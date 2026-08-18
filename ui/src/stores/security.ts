/**
 * The Security tab — the audit run and its report (PRD P1-1, SECURITY §6).
 *
 * Its own file, re-exported from `@/stores`, for the reason `health.ts` is: a domain that owns a
 * screen owns a file.
 *
 * Simpler than `health.ts` in two ways worth naming, because both are absences rather than
 * omissions. There are **no tabs** — one tool means one list, and a tab strip over a single tool
 * would be furniture. And the reset key is a **single** `envHash` rather than health's
 * `{envHash, folder}` pair, because an audit has no project folder: what it reads is the
 * environment itself.
 *
 * It gains one thing health has not: a **cancel**. An audit measures 18-68 s against Code Health's
 * 1.3 s, which is what changed the answer P4 recorded.
 */

import { create } from 'zustand'

import {
  auditCancel,
  auditRun,
  isPdError,
  onAuditProgress,
  type AuditReport,
  type PdError,
  type ProgressEvent,
  type PyEnv,
} from '@/ipc'
import { apply, type ConsoleLine } from '@/stores/plan'

/** Where a run is. */
export type SecurityPhase = 'idle' | 'running' | 'ready' | 'failed'

export interface SecurityState {
  phase: SecurityPhase
  /** The report on screen, or null when nothing has run for this environment. */
  report: AuditReport | null
  /**
   * Which environment `report` describes.
   *
   * Held separately so a report can never be shown under another environment's name — the same
   * reason `useHealthStore` keeps `reportFor`, and the reason `freshReport` exists below.
   */
  reportFor: string | null
  /** A run that failed outright, as opposed to a tool problem inside a report. */
  error: PdError | null
  /** Console lines from `audit-progress`, folded by the shared reducer. */
  console: ConsoleLine[]
  /**
   * Steps finished and expected.
   *
   * Required by `ConsoleState`, and not decoration: on a cold install `total` is the audit *plus*
   * the venv bootstrap's steps, so the drawer's "n of m" is the only thing on screen that says the
   * wait is a build rather than a hang.
   */
  done: number
  total: number
  current: string | null
  consoleOpen: boolean
  run: (env: PyEnv, envHash: string) => Promise<void>
  cancel: () => Promise<void>
  setConsoleOpen: (open: boolean) => void
  reset: () => void
}

/**
 * Cleared when the selected environment changes.
 *
 * `consoleOpen` is deliberately outside it: which drawer the user left open is a preference about
 * the screen, not a fact about an environment. Same call `useHealthStore` makes.
 */
const NO_SECURITY = {
  phase: 'idle' as SecurityPhase,
  report: null,
  reportFor: null,
  error: null,
  console: [] as ConsoleLine[],
  done: 0,
  total: 0,
  current: null,
}

const asPdError = (e: unknown): PdError =>
  isPdError(e) ? e : { code: 'PD-INT-001', message: String(e) }

let unlisten: (() => void) | undefined

export const useSecurityStore = create<SecurityState>((set, get) => ({
  ...NO_SECURITY,
  consoleOpen: false,

  run: async (env, envHash) => {
    if (get().phase === 'running') return

    // Cleared before subscribing, so a second Run never paints the previous advisories under a new
    // `ranAt`, and a report the server is about to replace stops being on screen first.
    set({ ...NO_SECURITY, phase: 'running' })

    // Subscribed before the command. On a cold install the audit venv's bootstrap is the slowest
    // part of the whole run, and it is emitted before `audit_run` returns anything at all.
    unlisten?.()
    unlisten = await onAuditProgress((event: ProgressEvent) => {
      useSecurityStore.setState((s) => apply(s, event))
    })

    try {
      const report = await auditRun(env)
      set({ report, reportFor: envHash, phase: 'ready', current: null })
    } catch (e) {
      set({ phase: 'failed', error: asPdError(e), current: null })
    } finally {
      unlisten?.()
      unlisten = undefined
    }
  },

  // Deliberately does **not** set a phase. The run's own `finally` resolves it, and a cancelled
  // audit still returns a report carrying `cancelled: true` — so setting `idle` here would race
  // the result and throw away the partial answer the user is entitled to see.
  cancel: async () => {
    if (get().phase !== 'running') return
    await auditCancel()
  },

  setConsoleOpen: (open) => {
    set({ consoleOpen: open })
  },

  reset: () => {
    unlisten?.()
    unlisten = undefined
    set({ ...NO_SECURITY })
  },
}))

/**
 * The report, but only if it belongs to this environment.
 *
 * A plain function rather than a hook so it stays trivially testable, exactly as
 * `useHealthStore`'s `freshReport` is — named apart from it because both are re-exported from
 * `@/stores` and one name cannot mean two shapes.
 */
export function freshAudit(
  state: Pick<SecurityState, 'report' | 'reportFor'>,
  envHash: string,
): AuditReport | null {
  return state.report !== null && state.reportFor === envHash ? state.report : null
}
