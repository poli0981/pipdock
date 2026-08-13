/**
 * Code Health — the run, the report, and the three tabs (CODE-HEALTH-SPEC §5).
 *
 * Its own file, re-exported from `@/stores`, for the reason `plan.ts` and `index-search.ts` are:
 * a domain that owns a screen owns a file.
 */

import { create } from 'zustand'

import {
  healthRun,
  isPdError,
  onHealthProgress,
  type HealthReport,
  type PdError,
  type ProgressEvent,
  type PyEnv,
  type RuffFinding,
} from '@/ipc'
import { apply, type ConsoleLine } from '@/stores/plan'

/** Which tab is showing. Deliberately the tool's own name — never translated (I18N §2). */
export type HealthTab = 'deptry' | 'vulture' | 'ruff'

/** Every tool a tab can render, in the order the tabs appear. */
export const HEALTH_TABS: readonly HealthTab[] = ['deptry', 'vulture', 'ruff'] as const

/** What a run is doing. `failed` is on the list for `PlanPhase`'s reason — see below. */
export type HealthPhase = 'idle' | 'running' | 'ready' | 'failed'

/** ruff's findings for one file, grouped once when the report lands. */
export interface RuffFileGroup {
  /** Absolute, as ruff reported it. Data — never localized. */
  file: string
  findings: RuffFinding[]
  /** How many of them ruff would actually fix. Display only; never summed into a total. */
  fixable: number
}

/**
 * Which environment and folder a report describes.
 *
 * **A pair, not a single key.** The folder is remembered *per environment* (CODE-HEALTH-SPEC §3)
 * and deptry's question is "declared here, against installed there" — so two environments can
 * legitimately name the same folder and mean different reports. Keying on either half alone shows
 * one environment's findings under another's name.
 */
export interface HealthKey {
  envHash: string
  folder: string
}

/**
 * Group ruff's findings by file, preserving the order it reported them in.
 *
 * **Pure, and called once when the report lands — never from a selector.** Zustand v5 sits on
 * `useSyncExternalStore`, so a selector building this array hands React a new reference on every
 * `getSnapshot` and either warns about caching or re-renders forever. `stores/index.ts` holds
 * `packages` as state for exactly this reason; this is the same rule, second application.
 */
export function groupRuff(findings: readonly RuffFinding[]): RuffFileGroup[] {
  const byFile = new Map<string, RuffFileGroup>()
  for (const finding of findings) {
    let group = byFile.get(finding.filename)
    if (group === undefined) {
      group = { file: finding.filename, findings: [], fixable: 0 }
      byFile.set(finding.filename, group)
    }
    group.findings.push(finding)
    // Only `safe` — `unsafe` and `display` are what `--unsafe-fixes` exists to opt into and
    // PipDock never passes it, so counting them would promise a fix the fix will not deliver.
    if (finding.fix === 'safe') group.fixable += 1
  }
  return [...byFile.values()]
}

/**
 * What a tab can be showing, which is three states rather than two.
 *
 * **`ran` alone is not enough, and this is the trap.** `health::run` fills `ran` from the
 * selection *before* the tool loop, so a tool that failed is in `ran`, is in `problems`, and
 * reports nothing. A tab keyed on `ran.includes(tool)` therefore renders a quarantined `ruff.exe`
 * as *no lint findings* — which is P3's own exit-criterion scenario, rendered as a lie.
 */
export type TabState = 'notRun' | 'failed' | 'clean' | 'findings'

/** Which of the three states `tool`'s tab is in, given the report and how many it found. */
export function tabState(report: HealthReport, tool: HealthTab, count: number): TabState {
  if (!report.ran.includes(tool)) return 'notRun'
  if ((report.problems ?? []).some((p) => p.tool === tool)) return 'failed'
  return count === 0 ? 'clean' : 'findings'
}

interface HealthState {
  /**
   * The project folder, or null before one has been chosen.
   *
   * Half the key, so it is deliberately **not** in `NO_HEALTH`: resetting it would throw away the
   * thing the reset is keyed on.
   */
  folder: string | null
  /** Which environment and folder `report` describes, or null when there is no report. */
  reportFor: HealthKey | null
  report: HealthReport | null
  /** ruff grouped by file, held as state. See `groupRuff`. */
  ruffByFile: RuffFileGroup[]
  tab: HealthTab
  phase: HealthPhase
  /**
   * A run that failed outright — not a tool that failed, which lands in `report.problems`.
   *
   * `phase: 'failed'` exists for the reason `PlanPhase.failed` does: a store that returned to
   * `idle` on error left no screen reading this field, so the user pressed Run and nothing
   * whatsoever happened.
   */
  error: PdError | null

  // -- the console slice, folded by the shared `apply` ----------------------------------------
  console: ConsoleLine[]
  done: number
  total: number
  current: string | null
  consoleOpen: boolean

  setFolder: (folder: string) => void
  setTab: (tab: HealthTab) => void
  setConsoleOpen: (open: boolean) => void
  run: (env: PyEnv, envHash: string) => Promise<void>
  reset: () => void
}

/**
 * Everything the health slice resets to.
 *
 * **The third reset key in this app, and the first that is a pair.** `NO_PACKAGES` is keyed to
 * `selected` and `NO_SNAPSHOTS` to `openFor`; those two were folded together once and a rescan
 * wiped a freshly-loaded timeline. A report is keyed to `(envHash, folder)`, which moves
 * independently of both.
 *
 * `folder` is not in here — it is half the key. `consoleOpen` is not either: whether the drawer is
 * open is a preference about the screen, not a property of a report.
 */
const NO_HEALTH = {
  reportFor: null,
  report: null,
  ruffByFile: [] as RuffFileGroup[],
  tab: 'deptry' as HealthTab,
  phase: 'idle' as HealthPhase,
  error: null,
  console: [] as ConsoleLine[],
  done: 0,
  total: 0,
  current: null,
}

/** Turn an unknown rejection into something a screen can render with a code. */
function asPdError(e: unknown): PdError {
  return isPdError(e) ? e : { code: 'PD-INT-001', message: String(e) }
}

/** Subscription to `health-progress`, at module scope so a re-run cannot leak the previous one. */
let unlisten: (() => void) | undefined

export const useHealthStore = create<HealthState>((set, get) => ({
  folder: null,
  consoleOpen: false,
  ...NO_HEALTH,

  setFolder: (folder) =>
    set((state) => (state.folder === folder ? {} : { folder, ...NO_HEALTH })),

  setTab: (tab) => set({ tab }),

  setConsoleOpen: (open) => set({ consoleOpen: open }),

  run: async (env, envHash) => {
    const { folder, phase } = get()
    if (folder === null || phase === 'running') return

    // Cleared *before* subscribing, so a second Run never paints the previous findings under a
    // new `ranAt` — and so a report the server is about to replace stops being on screen.
    set({ ...NO_HEALTH, phase: 'running' })

    // Subscribed before the command, or the first steps — the tools-venv bootstrap, on a cold
    // install the slowest part of the whole run — are emitted into nothing.
    unlisten?.()
    unlisten = await onHealthProgress((event: ProgressEvent) => {
      useHealthStore.setState((s) => apply(s, event))
    })

    try {
      const report = await healthRun(env, folder)
      set({
        report,
        reportFor: { envHash, folder },
        ruffByFile: groupRuff(report.ruff.findings),
        phase: 'ready',
        current: null,
      })
    } catch (e) {
      set({ phase: 'failed', error: asPdError(e), current: null })
    } finally {
      unlisten?.()
      unlisten = undefined
    }
  },

  reset: () => {
    unlisten?.()
    unlisten = undefined
    set({ ...NO_HEALTH })
  },
}))

/**
 * The report, but only when it describes the environment and folder currently on screen.
 *
 * A plain `report` read would show one environment's findings after the user switched to another —
 * the "never render a state you have not loaded" rule, in its less obvious direction. Kept as a
 * function rather than a hook so it stays trivially testable.
 */
export function freshReport(
  state: Pick<HealthState, 'report' | 'reportFor' | 'folder'>,
  envHash: string,
): HealthReport | null {
  const { report, reportFor, folder } = state
  if (report === null || reportFor === null) return null
  return reportFor.envHash === envHash && reportFor.folder === folder ? report : null
}
