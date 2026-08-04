/**
 * Zustand stores — ARCHITECTURE §9.
 *
 * Five domain stores mirror the five domains the screens read: `useEnvStore`, `usePlanStore`,
 * `useIndexStore`, `useSettingsStore`, `useHealthStore`. All engine data enters through the typed
 * IPC wrappers in `@/ipc`; stores never call Tauri themselves.
 *
 * `useUiStore` is a sixth, and a documented deviation from that list: which tab is showing is
 * *shell* state, not a domain. It lives here rather than in component state because the sidebar,
 * the content area and the keyboard map all read it.
 *
 * `usePlanStore`, `useIndexStore` and `useHealthStore` land with their screens.
 */

import { create } from 'zustand'

import type { NavKey } from '@/components/nav'
import { FALLBACK_LOCALE, type Locale } from '@/i18n'
import {
  envScan,
  isPdError,
  legalConsentGet,
  legalConsentSet,
  pinAdd,
  pinList,
  pinRemove,
  pkgList,
  pkgOutdated,
  settingsGet,
  settingsSet,
  type Dist,
  type EnvRow,
  type OutdatedDist,
  type PdError,
  type Pin,
  type ScanProgress,
  type Settings,
} from '@/ipc'
import { joinRows, type LoadState, type PackageRow } from '@/screens/rows'

/** Which resolver is active; shown in the status line (UI-SPEC §3). */
export type EngineId = 'pip' | 'uv'

/** Turn an unknown rejection into something a screen can render with a code. */
function asPdError(e: unknown): PdError {
  return isPdError(e)
    ? e
    : // A rejection that is not a PdError can only be a bug in the bridge, and PD-INT-001 means
      // exactly that. One honest internal code beats a blank error row.
      { code: 'PD-INT-001', message: String(e) }
}

interface SettingsState {
  /** Active UI language. */
  locale: Locale
  /** Configured engine. First run pre-selects uv when it is on PATH (ARCHITECTURE §3). */
  engine: EngineId
  /**
   * PEP 668 override. Off by default and deliberately hard to turn on — SECURITY §3 requires
   * every mutating screen for such an env to show a persistent warning chip while it is set.
   */
  allowExternallyManaged: boolean
  /** True until the first `settings_get` resolves, so Settings never flashes defaults. */
  loading: boolean
  error: PdError | null
  load: () => Promise<void>
  save: (patch: Partial<Settings>) => Promise<void>
  setLocale: (locale: Locale) => void
}

export const useSettingsStore = create<SettingsState>((set, get) => ({
  locale: FALLBACK_LOCALE,
  engine: 'pip',
  allowExternallyManaged: false,
  loading: true,
  error: null,

  load: async () => {
    try {
      const s = await settingsGet()
      set({
        engine: s.engine,
        allowExternallyManaged: s.allowExternallyManaged,
        loading: false,
        error: null,
      })
    } catch (e) {
      set({ loading: false, error: asPdError(e) })
    }
  },

  save: async (patch) => {
    const { engine, allowExternallyManaged } = get()
    const next: Settings = {
      engine,
      allowExternallyManaged,
      // The store holds the *resolved* locale; `null` on the wire means "follow the OS", and only
      // an explicit choice in Settings should overwrite that.
      locale: null,
      ...patch,
    }
    try {
      const saved = await settingsSet(next)
      set({
        engine: saved.engine,
        allowExternallyManaged: saved.allowExternallyManaged,
        error: null,
      })
    } catch (e) {
      set({ error: asPdError(e) })
    }
  },

  setLocale: (locale) => set({ locale }),
}))

interface EnvState {
  rows: EnvRow[]
  /** Interpreter path of the selected environment, or null before one is chosen. */
  selected: string | null
  scanning: boolean
  /** Latest `scan-progress`, so the screen can say what discovery is doing. */
  progress: ScanProgress | null
  error: PdError | null
  scan: () => Promise<void>
  setProgress: (progress: ScanProgress) => void
  select: (interpreter: string) => void

  // -- the package slice, for Installed and Updates ------------------------------------------

  /** Raw responses, one field per command, kept so any one can be refreshed on its own. */
  dists: Dist[]
  outdated: OutdatedDist[]
  pins: Pin[]

  /**
   * The joined rows, held as **state rather than derived in a selector**.
   *
   * Zustand v5 sits on `useSyncExternalStore`, so a selector returning a freshly built array
   * hands React a new reference every call and either warns about `getSnapshot` caching or
   * re-renders forever. Recomputed by the actions below via the pure `joinRows`.
   */
  packages: PackageRow[]
  /** Outdated names with no installed row — see `joinRows`. */
  orphanOutdated: string[]
  /** Count for the sidebar badge. A primitive, so reading it needs no memo. */
  updatesCount: number
  /** Which interpreter `packages` describes, so switching tabs does not refetch. */
  loadedFor: string | null

  listing: LoadState
  listError: PdError | null
  /**
   * Tracked separately from `listing` because `pkg_outdated` is the networked half. A PyPI
   * failure must cost badges, not the table.
   */
  outdatedStatus: LoadState
  outdatedError: PdError | null

  selection: Set<string>

  /** `pkg_list` + `pin_list` for the selected environment. */
  loadPackages: () => Promise<void>
  /** `pkg_outdated`, retryable on its own without re-reading the environment. */
  loadOutdated: () => Promise<void>
  toggle: (name: string) => void
  selectAll: (names: readonly string[]) => void
  clearSelection: () => void
  togglePin: (name: string) => Promise<void>
}

/** Everything the package slice resets to. Named so a future field cannot be forgotten. */
const NO_PACKAGES = {
  dists: [] as Dist[],
  outdated: [] as OutdatedDist[],
  pins: [] as Pin[],
  packages: [] as PackageRow[],
  orphanOutdated: [] as string[],
  updatesCount: 0,
  loadedFor: null,
  listing: 'idle' as LoadState,
  listError: null,
  outdatedStatus: 'idle' as LoadState,
  outdatedError: null,
  selection: new Set<string>(),
}

/** Recompute the joined view from the three raw responses. */
function joined(dists: Dist[], outdated: OutdatedDist[], pins: Pin[]) {
  const { rows, orphanOutdated } = joinRows(dists, outdated, pins)
  return {
    packages: rows,
    orphanOutdated,
    updatesCount: rows.filter((r) => r.latest !== undefined).length,
  }
}

export const useEnvStore = create<EnvState>((set, get) => ({
  rows: [],
  selected: null,
  scanning: false,
  progress: null,
  error: null,
  ...NO_PACKAGES,

  scan: async () => {
    // Discovery spawns probe.py once per candidate interpreter, so a duplicate scan is the most
    // expensive thing the app can do twice. `PdEnvironments` has started two on every mount since
    // Stage 1 — StrictMode runs effects twice in development — which was invisible because both
    // produce the same rows.
    if (get().scanning) return
    set({ scanning: true, error: null })
    try {
      const rows = await envScan()
      set((state) => {
        const selected =
          // Keep the current selection if it survived the rescan; otherwise fall back to the first
          // usable row, so the status line is never empty while something is available.
          state.selected !== null && rows.some((r) => r.interpreter === state.selected)
            ? state.selected
            : (rows.find((r) => r.env !== undefined)?.interpreter ?? null)
        return {
          rows,
          scanning: false,
          progress: null,
          selected,
          // A rescan can move the selection, and the package slice belongs to whichever
          // environment was selected when it was read.
          ...(selected === state.loadedFor ? {} : NO_PACKAGES),
        }
      })
    } catch (e) {
      set({ scanning: false, progress: null, error: asPdError(e) })
    }
  },

  setProgress: (progress) => set({ progress }),

  // Clearing the package slice here is load-bearing: without it, switching environments shows the
  // previous one's packages under the new one's name. Invisible to any test that loads one env.
  select: (interpreter) =>
    set((state) =>
      state.selected === interpreter ? {} : { selected: interpreter, ...NO_PACKAGES },
    ),

  loadPackages: async () => {
    const { selected, rows, listing } = get()
    const row = rows.find((r) => r.interpreter === selected)
    if (selected === null || row?.env === undefined) return
    // Already in flight. `loadedFor` is only set once the await resolves, so a screen effect
    // firing twice before then — which StrictMode does on every mount in development — would
    // otherwise spawn probe.py twice for the same environment. That is the exact cost the
    // RECORD-parsing work in the probe existed to avoid.
    if (listing === 'loading') return

    set({ listing: 'loading', listError: null })
    try {
      const [dists, pins] = await Promise.all([pkgList(row.env), pinList(row.envHash)])
      set({ dists, pins, ...joined(dists, get().outdated, pins), listing: 'ready', loadedFor: selected })
    } catch (e) {
      set({ listing: 'error', listError: asPdError(e) })
    }
  },

  loadOutdated: async () => {
    const { selected, rows, outdatedStatus } = get()
    const row = rows.find((r) => r.interpreter === selected)
    if (selected === null || row?.env === undefined) return
    // Same guard as `loadPackages`, and it matters more here: this one hits the network.
    if (outdatedStatus === 'loading') return

    set({ outdatedStatus: 'loading', outdatedError: null })
    try {
      const outdated = await pkgOutdated(row.env)
      set({ outdated, ...joined(get().dists, outdated, get().pins), outdatedStatus: 'ready' })
    } catch (e) {
      // The installed table stays; only the badges are missing. This is the whole reason
      // `pkg_list` and `pkg_outdated` are two commands.
      set({ outdatedStatus: 'error', outdatedError: asPdError(e) })
    }
  },

  toggle: (name) =>
    set((state) => {
      const next = new Set(state.selection)
      if (!next.delete(name)) next.add(name)
      return { selection: next }
    }),

  selectAll: (names) => set({ selection: new Set(names) }),
  clearSelection: () => set({ selection: new Set<string>() }),

  togglePin: async (name) => {
    const { selected, rows, pins, dists, outdated } = get()
    const row = rows.find((r) => r.interpreter === selected)
    if (row === undefined) return

    const existing = pins.find((p) => p.pkg === name)
    try {
      if (existing === undefined) {
        // From a row, a pin is always `Exclude` — "do not sweep this up". Holding at a version
        // needs a reason field, which UI-SPEC §4 puts on the Pins screen (S5).
        await pinAdd(row.envHash, { pkg: name, mode: 'exclude' })
      } else {
        await pinRemove(row.envHash, name)
      }
      const fresh = await pinList(row.envHash)
      set((state) => ({
        pins: fresh,
        ...joined(dists, outdated, fresh),
        // A newly pinned package must leave the selection, or Select all's count and what is
        // actually ticked disagree.
        selection: new Set([...state.selection].filter((n) => !fresh.some((p) => p.pkg === n))),
      }))
    } catch (e) {
      set({ listError: asPdError(e) })
    }
  },
}))

interface LegalState {
  /** Null until checked; false means the gate must be shown. */
  accepted: boolean | null
  error: PdError | null
  check: () => Promise<void>
  accept: () => Promise<void>
}

export const useLegalStore = create<LegalState>((set) => ({
  accepted: null,
  error: null,

  check: async () => {
    try {
      const state = await legalConsentGet()
      set({ accepted: state.current, error: null })
    } catch (e) {
      // A consent record that cannot be read must not skip the gate. Failing closed is the only
      // safe direction here.
      set({ accepted: false, error: asPdError(e) })
    }
  },

  accept: async () => {
    try {
      await legalConsentSet()
      set({ accepted: true, error: null })
    } catch (e) {
      set({ error: asPdError(e) })
    }
  },
}))

interface UiState {
  nav: NavKey
  setNav: (nav: NavKey) => void
}

export const useUiStore = create<UiState>((set) => ({
  nav: 'environments',
  setNav: (nav) => set({ nav }),
}))

// The plan store lives in its own file: it is the largest of them and the only one holding a live
// event subscription. Re-exported so `@/stores` stays the single import site.
export { CONSOLE_LIMIT, apply, usePlanStore, type ConsoleLine, type PlanPhase } from '@/stores/plan'
