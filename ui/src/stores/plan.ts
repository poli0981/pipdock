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
  isPdError,
  onPlanProgress,
  planCancel,
  planDecide,
  planExecute,
  planResolve,
  type Decision,
  type ExecutionOutcome,
  type FlowStep,
  type Intent,
  type PdError,
  type ProgressEvent,
  type PyEnv,
} from '@/ipc'

/**
 * How many console lines to keep.
 *
 * A source build is tens of thousands of lines and every one of them would otherwise be a React
 * state update holding a string alive. The drawer shows a tail; the *complete* log is the log
 * file, and ERROR-CATALOG §3's "Copy full log" reads that rather than this.
 */
export const CONSOLE_LIMIT = 2000

/** Which part of DATA-FLOW §3 is on screen. */
export type PlanPhase = 'idle' | 'resolving' | 'preview' | 'executing' | 'summary'

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
  cancel: () => Promise<void>
  setConsoleOpen: (open: boolean) => void
  /** Return to the table, discarding the preview or the summary. */
  reset: () => void
}

function asPdError(e: unknown): PdError {
  return isPdError(e) ? e : { code: 'PD-INT-001', message: String(e) }
}

const EMPTY = {
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

export const usePlanStore = create<PlanState>((set, get) => ({
  phase: 'idle',
  consoleOpen: false,
  ...EMPTY,

  resolve: async (env, intent) => {
    set({ phase: 'resolving', ...EMPTY })
    try {
      set({ step: await planResolve(env, intent), phase: 'preview' })
    } catch (e) {
      set({ phase: 'idle', error: asPdError(e) })
    }
  },

  choose: (pkg, decision) =>
    set((s) => ({ decisions: { ...s.decisions, [pkg]: decision } })),

  submitDecisions: async () => {
    const { decisions } = get()
    set({ phase: 'resolving', error: null })
    try {
      // Answers are cleared with the round they belonged to: the next preview may name a
      // different set of packages, and a stale answer for one no longer in conflict would be
      // sent back to a flow that has moved on.
      set({ step: await planDecide(decisions), decisions: {}, phase: 'preview' })
    } catch (e) {
      set({ phase: 'preview', error: asPdError(e) })
    }
  },

  execute: async () => {
    set({ phase: 'executing', console: [], done: 0, total: 0, error: null, cancelling: false })

    // Subscribed before the command, or the first steps are emitted into nothing.
    unlisten?.()
    unlisten = await onPlanProgress((event: ProgressEvent) => {
      set((s) => apply(s, event))
    })

    try {
      const outcome = await planExecute()
      set({ outcome, phase: 'summary', current: null })
    } catch (e) {
      set({ phase: 'summary', error: asPdError(e), current: null })
    } finally {
      unlisten?.()
      unlisten = undefined
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
    unlisten?.()
    unlisten = undefined
    set({ phase: 'idle', ...EMPTY })
  },
}))

/** Fold one progress event into the store. Pure, so the lifecycle rules are testable. */
export function apply(state: PlanState, event: ProgressEvent): Partial<PlanState> {
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
