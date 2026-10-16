/**
 * The mutation flow, as the screen sees it — ARCHITECTURE §9's `usePlanStore`.
 *
 * Mirrors DATA-FLOW §3's state machine, which is deliberately *not* re-implemented here: the flow
 * itself lives in Rust and is resumable, and this store only tracks which of its steps is on
 * screen. Every transition is a command; nothing here decides what is allowed next.
 *
 * ```text
 *   idle ──resolve()──► resolving ──► preview ──decide()──► preview
 *                            │            │                    │
 *                            │            └──execute()──► executing ──► summary
 *                            └──► error                        │
 *                                                        cancel() trips the token;
 *                                                        the summary still arrives
 * ```
 *
 * In a separate file from the other stores because it is the largest of them and the only one
 * holding a live event subscription; `@/stores` still re-exports it, so there is one import site.
 */

import { create } from 'zustand'

import {
  onPlanProgress,
  planCancel,
  planDecide,
  planExecute,
  planResolve,
  snapshotRollback,
  snapshotRollbackPreview,
  uninstallExecute,
  uninstallGuard,
  type Decision,
  type ExecutionOutcome,
  type FlowStep,
  type GuardReport,
  type Intent,
  type PdError,
  type ProgressEvent,
  type PyEnv,
  type RollbackPreview,
} from '@/ipc'
import { asPdError } from '@/ipc/errors'

/**
 * How many console lines to keep.
 *
 * A source build is tens of thousands of lines and every one of them would otherwise be a React
 * state update holding a string alive. The drawer shows a tail; the *complete* log is the log
 * file, and ERROR-CATALOG §3's "Copy full log" reads that rather than this.
 */
export const CONSOLE_LIMIT = 2000

/** Which part of DATA-FLOW §3 (or §5) is on screen. */
export type PlanPhase =
  | 'idle'
  | 'resolving'
  | 'preview'
  /** The uninstall guard is running; the dialog opens over the table when it answers. */
  | 'guarding'
  | 'guard'
  | 'executing'
  | 'summary'
  | 'failed'

/**
 * The phases the plan panel is on screen for.
 *
 * `idle` is not one, obviously. `failed` **is**, and that is the point: a resolve that threw used
 * to set `idle`, which un-mounted the panel — and no screen reads `error`, so `PD-RES-003`, a PEP
 * 668 refusal and every engine failure during resolve vanished without a trace. The user pressed
 * Update and nothing whatsoever happened.
 *
 * `guard` is deliberately **not** one either: DATA-FLOW §5's dialog opens *over* the table the
 * user selected from, so they can still see what they picked while deciding. Nor is `guarding`,
 * the probe before it: the guard used to set `resolving`, which swapped the table for the plan
 * panel for half a second and threw away the table's scroll position (U14).
 */
export const PANEL_PHASES: ReadonlySet<PlanPhase> = new Set<PlanPhase>([
  'resolving',
  'preview',
  'executing',
  'summary',
  'failed',
])

/**
 * Which mutation this session is.
 *
 * One store, not two. A removal has no `ResolutionReport` and so no preview, but it produces the
 * same `plan-progress` stream and the same `ExecutionSummary` — and there is one session at a time
 * in Rust either way, so two stores would be two front ends onto one slot, each able to think it
 * owns it.
 */
export type PlanKind = 'update' | 'install' | 'uninstall' | 'rollback'

/** One console line, already flattened for rendering. */
export interface ConsoleLine {
  /** Step index, so the drawer can group by section. */
  step: number
  /** The package, when the step is for one. */
  pkg?: string
  /** `stderr` renders differently — but is not an error (uv writes its plan there). */
  stream: 'stdout' | 'stderr'
  text: string
}

interface PlanState {
  phase: PlanPhase
  /** Which mutation is in flight, or null when there is none. */
  kind: PlanKind | null
  /** The environment it is acting on. Retained because the guard is re-run against it. */
  env: PyEnv | null
  /** What the guard found, while `phase` is `guard`. */
  guard: GuardReport | null
  /** What a rollback would do, while `kind` is `rollback` — DATA-FLOW §8. */
  preview: RollbackPreview | null
  /** True while a re-guard is in flight, so the dialog can say so rather than flicker. */
  guardBusy: boolean
  /** What the flow says it needs next, or null before the first resolve. */
  step: FlowStep | null
  /** The user's answers so far, keyed by package name. */
  decisions: Record<string, Decision>
  /** Bounded tail of engine output. */
  console: ConsoleLine[]
  /** Section markers, for the drawer's headings and the live region's counter. */
  done: number
  total: number
  /** The package currently installing, for the status line. */
  current: string | null
  outcome: ExecutionOutcome | null
  error: PdError | null
  /** Whether the drawer is showing. Shell state, but it belongs with what fills it. */
  consoleOpen: boolean
  /** True once cancel has been asked for, so the button can stop offering. */
  cancelling: boolean

  resolve: (env: PyEnv, intent: Intent) => Promise<void>
  choose: (pkg: string, decision: Decision) => void
  submitDecisions: () => Promise<void>
  execute: () => Promise<void>
  /** Run the guard over `pkgs` and open the dialog (DATA-FLOW §5). */
  startUninstall: (env: PyEnv, pkgs: string[]) => Promise<void>
  /** *Remove dependents too*: re-guard over the widened set rather than proceeding. */
  widen: () => Promise<void>
  /** Remove. `force` is *Force remove only X*. */
  confirmUninstall: (force: boolean) => Promise<void>
  /** Preview restoring `id`, which parks the flow that would do it (DATA-FLOW §8). */
  rollback: (env: PyEnv, id: string) => Promise<void>
  cancel: () => Promise<void>
  setConsoleOpen: (open: boolean) => void
  /** Return to the table, discarding the preview or the summary. */
  reset: () => void
}

const EMPTY = {
  kind: null,
  env: null,
  guard: null,
  guardBusy: false,
  preview: null,
  step: null,
  decisions: {},
  console: [] as ConsoleLine[],
  done: 0,
  total: 0,
  current: null,
  outcome: null,
  error: null,
  cancelling: false,
}

/**
 * Unlisten for the current execution's progress subscription.
 *
 * Module-level rather than in the store: it is not state any component renders, and putting a
 * function in the store would make every subscriber re-render when it changed.
 */
let unlisten: (() => void) | undefined

/**
 * Which plan the store is on. Every command's answer is checked against it, and `reset` moves it
 * on — so an answer that arrives after the user left is dropped instead of re-opening the panel
 * they just closed (U9).
 */
let generation = 0

/**
 * How long to keep listening after the command answers. The command's reply and its last progress
 * events travel separately and in no promised order, and unsubscribing at once lost the final
 * `stepFinished` — the live region stopped one short of the total.
 */
const TRAILING_EVENTS_MS = 50

/**
 * Drive one execution to its summary: subscribe, run, unsubscribe.
 *
 * Shared by `execute` and `confirmUninstall` because the tail of DATA-FLOW §3 and §5 is the same
 * tail — the same `plan-progress` stream, the same `ExecutionSummary`, the same console drawer and
 * summary sheet. Only the command differs, so only the command is a parameter.
 */
async function runToSummary(
  set: (partial: Partial<PlanState>) => void,
  command: () => Promise<ExecutionOutcome>,
): Promise<void> {
  set({ phase: 'executing', console: [], done: 0, total: 0, error: null, cancelling: false })

  // Subscribed before the command, or the first steps are emitted into nothing.
  unlisten?.()
  unlisten = await onPlanProgress((event: ProgressEvent) => {
    usePlanStore.setState((s) => apply(s, event))
  })

  const mine = generation
  try {
    const outcome = await command()
    await new Promise((r) => setTimeout(r, TRAILING_EVENTS_MS))
    if (mine !== generation) return
    set({ outcome, phase: 'summary', current: null })
  } catch (e) {
    if (mine !== generation) return
    set({ phase: 'summary', error: asPdError(e), current: null })
  } finally {
    if (mine === generation) {
      unlisten?.()
      unlisten = undefined
    }
  }
}

export const usePlanStore = create<PlanState>((set, get) => ({
  phase: 'idle',
  consoleOpen: false,
  ...EMPTY,

  resolve: async (env, intent) => {
    const mine = ++generation
    set({ phase: 'resolving', ...EMPTY, kind: intent.intent === 'install' ? 'install' : 'update', env })
    try {
      const step = await planResolve(env, intent)
      if (mine !== generation) return
      set({ step, phase: 'preview' })
    } catch (e) {
      if (mine !== generation) return
      // `failed`, not `idle`: the panel is what renders `error`, and going back to `idle`
      // un-mounts it. Every resolve failure — a plan already in flight, a PEP 668 environment,
      // an unreachable index — was silently swallowed by that.
      set({ phase: 'failed', error: asPdError(e) })
    }
  },

  choose: (pkg, decision) =>
    set((s) => ({ decisions: { ...s.decisions, [pkg]: decision } })),

  submitDecisions: async () => {
    const { decisions } = get()
    const mine = generation
    set({ phase: 'resolving', error: null })
    try {
      // Answers are cleared with the round they belonged to: the next preview may name a
      // different set of packages, and a stale answer for one no longer in conflict would be
      // sent back to a flow that has moved on.
      const step = await planDecide(decisions)
      if (mine !== generation) return
      set({ step, decisions: {}, phase: 'preview' })
    } catch (e) {
      if (mine !== generation) return
      // The preview stays and stays usable: a failed round leaves the flow as it was (U24).
      set({ phase: 'preview', error: asPdError(e) })
    }
  },

  execute: async () => {
    // One button, two commands. `PdPlanPanel`'s Confirm is the same control for both flows, and
    // which one it sends is the session's kind rather than anything the button knows.
    const command = get().kind === 'rollback' ? snapshotRollback : planExecute
    await runToSummary(set, command)
  },

  startUninstall: async (env, pkgs) => {
    const mine = ++generation
    set({ phase: 'guarding', ...EMPTY, kind: 'uninstall', env })
    try {
      const guard = await uninstallGuard(env, pkgs)
      if (mine !== generation) return
      set({ guard, phase: 'guard' })
    } catch (e) {
      if (mine !== generation) return
      set({ phase: 'failed', error: asPdError(e) })
    }
  },

  widen: async () => {
    const { env, guard } = get()
    if (env === null || guard === null) return

    // The whole point of DATA-FLOW §5's re-guard: the widened set goes back through the guard
    // instead of straight to execution, because a dependent of a dependent has to surface before
    // it is removed. `guardBusy` rather than leaving the dialog blank — it usually comes back
    // inside a probe's worth of time and a flicker reads as a bug.
    const mine = generation
    set({ guardBusy: true, error: null })
    try {
      const widened = await uninstallGuard(env, guard.withDependents)
      if (mine !== generation) return
      set({ guard: widened, phase: 'guard' })
    } catch (e) {
      if (mine !== generation) return
      set({ phase: 'failed', error: asPdError(e) })
    } finally {
      if (mine === generation) set({ guardBusy: false })
    }
  },

  confirmUninstall: async (force) => {
    await runToSummary(set, () => uninstallExecute(force))
  },

  rollback: async (env, id) => {
    const mine = ++generation
    set({ phase: 'resolving', ...EMPTY, kind: 'rollback', env })
    try {
      const preview = await snapshotRollbackPreview(env, id)
      if (mine !== generation) return
      set({ preview, phase: 'preview' })
    } catch (e) {
      if (mine !== generation) return
      set({ phase: 'failed', error: asPdError(e) })
    }
  },

  cancel: async () => {
    set({ cancelling: true })
    try {
      await planCancel()
    } catch (e) {
      // Failing to cancel is worth showing, but the execution is still running and its summary
      // is still coming — so the phase does not change.
      set({ error: asPdError(e) })
    }
  },

  setConsoleOpen: (open) => set({ consoleOpen: open }),

  reset: () => {
    // Leaving a plan that the backend is still working on — a resolve, a guard probe — or holding
    // parked — a preview, a guard dialog — stops it there too. Back during a resolve used to
    // return to the table while pip went on backtracking with the slot claimed, and the answer
    // then re-opened the panel the user had just closed (U9).
    const { phase } = get()
    if (phase === 'resolving' || phase === 'guarding' || phase === 'preview' || phase === 'guard') {
      void planCancel().catch(() => undefined)
    }
    generation += 1
    unlisten?.()
    unlisten = undefined
    set({ phase: 'idle', ...EMPTY })
  },
}))

/**
 * The slice of a store that a `ProgressEvent` stream writes to.
 *
 * Named so `apply` can serve two stores rather than being copied into the second. Health streams
 * `health-progress`, which carries the **identical** `ProgressEvent` payload — so the alternative
 * was a second reducer with its own `CONSOLE_LIMIT`, its own drop-from-the-front rule and its own
 * opportunity to get `done` wrong.
 */
export interface ConsoleState {
  console: ConsoleLine[]
  done: number
  total: number
  current: string | null
}

/**
 * Fold one progress event into the console slice of a store. Pure, so the lifecycle rules are
 * testable — and generic, so `usePlanStore` and `useHealthStore` cannot drift apart.
 */
export function apply<S extends ConsoleState>(state: S, event: ProgressEvent): Partial<ConsoleState> {
  switch (event.kind) {
    case 'stepStarted':
      return { total: event.total, current: event.pkg ?? null }
    case 'stepFinished':
      // The live region counts closes, not lines — which is the whole reason the payload became
      // a lifecycle. `done` can only go up.
      return { total: event.total, done: state.done + 1, current: null }
    case 'line': {
      const next = [
        ...state.console,
        {
          step: event.step,
          ...(event.pkg == null ? {} : { pkg: event.pkg }),
          stream: event.stream,
          text: event.line,
        },
      ]
      // Drop from the front rather than refusing to grow: the tail is what a user watching a
      // build wants to see.
      return { console: next.length > CONSOLE_LIMIT ? next.slice(-CONSOLE_LIMIT) : next }
    }
  }
}
