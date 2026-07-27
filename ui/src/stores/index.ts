/**
 * Zustand stores — ARCHITECTURE §9.
 *
 * Five stores mirror the five domains the screens read: `useEnvStore`, `usePlanStore`,
 * `useIndexStore`, `useSettingsStore`, `useHealthStore`. All engine data enters through the typed
 * IPC wrappers in `@/ipc`; stores never call Tauri themselves.
 *
 * Only the settings store exists in Phase 0, because the app shell already needs a locale and an
 * engine badge. The rest land in M2 with their screens.
 */

import { create } from 'zustand'

import { FALLBACK_LOCALE, type Locale } from '@/i18n'

/** Which resolver is active; shown in the status line (UI-SPEC §3). */
export type EngineId = 'pip' | 'uv'

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
  setLocale: (locale: Locale) => void
  setEngine: (engine: EngineId) => void
}

export const useSettingsStore = create<SettingsState>((set) => ({
  locale: FALLBACK_LOCALE,
  engine: 'pip',
  allowExternallyManaged: false,
  setLocale: (locale) => set({ locale }),
  setEngine: (engine) => set({ engine }),
}))
