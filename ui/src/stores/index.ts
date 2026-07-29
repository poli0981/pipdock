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
  settingsGet,
  settingsSet,
  type EnvRow,
  type PdError,
  type ScanProgress,
  type Settings,
} from '@/ipc'

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
}

export const useEnvStore = create<EnvState>((set) => ({
  rows: [],
  selected: null,
  scanning: false,
  progress: null,
  error: null,

  scan: async () => {
    set({ scanning: true, error: null })
    try {
      const rows = await envScan()
      set((state) => ({
        rows,
        scanning: false,
        progress: null,
        // Keep the current selection if it survived the rescan; otherwise fall back to the first
        // usable row, so the status line is never empty while something is available.
        selected:
          state.selected !== null && rows.some((r) => r.interpreter === state.selected)
            ? state.selected
            : (rows.find((r) => r.env !== undefined)?.interpreter ?? null),
      }))
    } catch (e) {
      set({ scanning: false, progress: null, error: asPdError(e) })
    }
  },

  setProgress: (progress) => set({ progress }),
  select: (interpreter) => set({ selected: interpreter }),
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
