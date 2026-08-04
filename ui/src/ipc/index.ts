/**
 * The typed IPC boundary — ARCHITECTURE §9.
 *
 * **No component calls `invoke` directly.** Everything crosses through this module, which is the
 * one place `@tauri-apps/api/core` may be imported (enforced by `no-restricted-imports` in
 * `eslint.config.js`).
 *
 * Data types are generated from the Rust definitions into `./generated.ts` by
 * `cargo run -p xtask -- bindings`, and `cargo test` fails while that file is stale. Command
 * *signatures* are not derivable from a schema, so the wrappers below are hand-written, one per
 * entry in `COMMANDS`.
 */

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

import type { Dist, EngineId, EnvSource, OutdatedDist, Pin, PyEnv } from './generated'

export type * from './generated'

/** Every Tauri command, per ARCHITECTURE §7. All are async and all return `Result<T, PdError>`. */
export const COMMANDS = [
  'app_info',
  'env_scan',
  'env_add_manual',
  'env_probe',
  'pkg_list',
  'pkg_outdated',
  'index_search',
  'index_refresh',
  'pkg_metadata',
  'plan_resolve',
  'plan_decide',
  'plan_execute',
  'plan_cancel',
  'uninstall_guard',
  'uninstall_execute',
  'pin_list',
  'pin_add',
  'pin_remove',
  'snapshot_list',
  'snapshot_create',
  'snapshot_diff',
  'snapshot_rollback',
  'health_run',
  'health_fix',
  'engine_info',
  'pip_upgrade',
  'settings_get',
  'settings_set',
  'legal_consent_get',
  'legal_consent_set',
  'logs_tail',
  'report_bug_url',
] as const

export type Command = (typeof COMMANDS)[number]

/** Tauri event channels, per ARCHITECTURE §7. */
export const EVENTS = ['plan-progress', 'scan-progress', 'health-progress'] as const

export type EventName = (typeof EVENTS)[number]

/** The error shape every command rejects with; mirrors `pipdock_core::errors::PdError`. */
export interface PdError {
  /** Catalog code, e.g. `PD-BLD-002`. Never localized (docs/I18N.md §1). */
  code: string
  /** Developer-facing detail. The user-facing one-liner is looked up from `code`. */
  message: string
  /** Engine stderr tail, at most 40 lines (docs/ERROR-CATALOG.md §3). */
  stderrTail?: string
}

/**
 * True for anything that came back from a rejected command.
 *
 * Tauri rejects with whatever the Rust side serialized, so a `catch` receives an `unknown` that is
 * *usually* a `PdError` — but a panic or a serialization failure arrives as something else.
 * Narrowing here means no screen has to guess.
 */
export function isPdError(value: unknown): value is PdError {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as PdError).code === 'string' &&
    typeof (value as PdError).message === 'string'
  )
}

/** What the shell reports about itself. */
export interface AppInfo {
  version: string
  /** Hash of the legal documents this build ships against (UI-SPEC §4). */
  docsHash: string
}

/** One row of the Environments screen. A probe failure is per-row, never fatal to the scan. */
export interface EnvRow {
  interpreter: string
  source: EnvSource
  envHash: string
  env?: PyEnv
  packages?: number
  error?: PdError
}

/** Progress from a discovery sweep. `label` is a path and is never localized (I18N §2). */
export interface ScanProgress {
  phase: 'registry' | 'launcher' | 'uv' | 'venv-scan' | 'collating'
  done: number
  total: number
  label?: string
}

/** Settings, as stored (UI-SPEC §4). */
export interface Settings {
  engine: EngineId
  /** `null` means "follow the OS", which is what a fresh install does. */
  locale: string | null
  /** SECURITY §3: off by default, never inferred. */
  allowExternallyManaged: boolean
}

/** A recorded acceptance of the legal documents. */
export interface Consent {
  docsHash: string
  acceptedAt: string
}

/** Whether the legal gate can be skipped. */
export interface ConsentState {
  current: boolean
  docsHash: string
  recorded?: Consent
}

export const appInfo = (): Promise<AppInfo> => invoke('app_info')

export const envScan = (): Promise<EnvRow[]> => invoke('env_scan')

export const envProbe = (interpreter: string): Promise<EnvRow> =>
  invoke('env_probe', { interpreter })

/**
 * Everything installed in `env` — the Installed table.
 *
 * Pass back the `PyEnv` from `envScan`/`envProbe` rather than a bare path: the source chip
 * survives the round trip, and the Rust side does not have to guess it. Re-probes, so it is the
 * fresh listing rather than whatever the last scan saw.
 */
export const pkgList = (env: PyEnv): Promise<Dist[]> => invoke('pkg_list', { env })

/**
 * Installed packages with a newer release available, via the configured engine.
 *
 * Separate from `pkgList` because it is the one that touches the network. Render the installed
 * rows from `pkgList` first and badge them when this resolves — a failure here costs badges, not
 * the table.
 */
export const pkgOutdated = (env: PyEnv): Promise<OutdatedDist[]> => invoke('pkg_outdated', { env })

/** Pins for an environment, ordered by package name. `envHash` comes from `EnvRow`. */
export const pinList = (envHash: string): Promise<Pin[]> => invoke('pin_list', { envHash })

/** Add or replace a pin. Rejects with `PD-PKG-002` if the name or held version is malformed. */
export const pinAdd = (envHash: string, pin: Pin): Promise<void> =>
  invoke('pin_add', { envHash, pin })

/** Remove a pin, resolving to whether one existed. */
export const pinRemove = (envHash: string, pkg: string): Promise<boolean> =>
  invoke('pin_remove', { envHash, pkg })

export const settingsGet = (): Promise<Settings> => invoke('settings_get')

export const settingsSet = (settings: Settings): Promise<Settings> =>
  invoke('settings_set', { settings })

export const legalConsentGet = (): Promise<ConsentState> => invoke('legal_consent_get')

export const legalConsentSet = (): Promise<Consent> => invoke('legal_consent_set')

/** Subscribe to discovery progress. Returns the unlisten function. */
export const onScanProgress = (handler: (progress: ScanProgress) => void): Promise<UnlistenFn> =>
  listen<ScanProgress>('scan-progress', (event) => {
    handler(event.payload)
  })
