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

import type {
  Decision,
  Dist,
  Freshness,
  Hit,
  EngineId,
  EnvSource,
  Diff,
  EngineInfo,
  ExecutionSummary,
  FlowStep,
  GuardReport,
  Intent,
  OutdatedDist,
  PackageMeta,
  Pin,
  ProgressEvent,
  PyEnv,
  HealthReport,
  RefreshReport,
  RollbackPreview,
  SnapshotMeta,
  StepResult,
} from './generated'

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
  'snapshot_rollback_preview',
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
  /**
   * The environment's own pip, out of the probe's distribution list — not `engine_info`, which
   * would add two subprocesses per row to the landing screen.
   *
   * Absent means the probe found no pip, which is a real state (`--without-pip`, or `-I` hiding a
   * user-site install), not "not yet known".
   */
  pipVersion?: string
  /**
   * The project folder Code Health last ran in for this environment.
   *
   * Absent means Health has never run here — which is a real state the screen renders as *choose a
   * folder*, not "not yet known". Carried on the row rather than fetched: the same trade `pipVersion`
   * makes, and the reason P4 needed no command of its own.
   */
  healthProject?: string
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

/**
 * Re-probe one interpreter and rebuild its row.
 *
 * **Pass `source` when refreshing an existing row.** Omitting it means *Browse…*, and the row comes
 * back labelled `manual` — which is what used to happen to every row `upgradePip` refreshed,
 * silently relabelling a registry-discovered Python in the chip and in the `PyEnv` handed to every
 * later `pkgList`.
 */
export const envProbe = (interpreter: string, source?: EnvSource): Promise<EnvRow> =>
  invoke('env_probe', source === undefined ? { interpreter } : { interpreter, source })

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

/**
 * What `indexSearch` resolves to.
 *
 * `ready` is separate from an empty `hits` because "no such package" and "the index is still
 * loading" are different answers. Loading 864k names costs ~140 ms, and telling someone their
 * package does not exist for even that long would be a lie.
 */
export interface SearchResults {
  hits: Hit[]
  ready: boolean
  /** Set when the index cannot be loaded at all — usually "never refreshed". */
  unavailable?: string
}

/**
 * Fuzzy search the local name index. Safe to call on every keystroke.
 *
 * Never blocks on the index load: a search that arrives first comes back `ready: false` and the
 * screen says so. SP-3 ruled out the alternative — scanning SQLite per keystroke measured 218 ms
 * against a 50 ms budget.
 */
export const indexSearch = (query: string, limit: number): Promise<SearchResults> =>
  invoke('index_search', { query, limit })

/** Cached PyPI metadata for the details panel, with how fresh it is. */
export const pkgMetadata = (pkg: string): Promise<[PackageMeta, Freshness]> =>
  invoke('pkg_metadata', { pkg })

/** Re-download the PEP 691 name index. The previous index stays searchable if this fails. */
export const indexRefresh = (): Promise<RefreshReport> => invoke('index_refresh')

/** What `planExecute` resolves to: the summary, and the snapshot taken before anything ran. */
export interface ExecutionOutcome {
  summary: ExecutionSummary
  /** Absent only when the snapshot was waived, which the GUI never does. */
  snapshot?: SnapshotMeta
}

/**
 * Begin an update or install: resolve, and derive what needs a decision.
 *
 * The first of four calls that drive **one** resumable flow held in Rust between them. Rejects
 * with `PD-RES-003` if a plan is already in flight — one at a time, deliberately, because two
 * would interleave engine commands against a single environment.
 */
export const planResolve = (env: PyEnv, intent: Intent): Promise<FlowStep> =>
  invoke('plan_resolve', { env, intent })

/** Apply the 3-way conflict choices and re-resolve. Keyed by package name. */
export const planDecide = (decisions: Record<string, Decision>): Promise<FlowStep> =>
  invoke('plan_decide', { decisions })

/**
 * Snapshot, then run the plan. Streams `plan-progress` throughout.
 *
 * The snapshot is not optional: DATA-FLOW §9.2 aborts with `PD-SNP-001` rather than mutating
 * without one, and `--no-snapshot` is a CLI waiver with no GUI surface.
 */
export const planExecute = (): Promise<ExecutionOutcome> => invoke('plan_execute')

/**
 * Stop the running plan. Resolves to whether there was anything to stop.
 *
 * A plan that is merely parked — a preview on screen, a guard dialog open — counts and is
 * discarded: it has no process to kill, and leaving it refuses the next plan on behalf of
 * something nobody is looking at.
 */
export const planCancel = (): Promise<boolean> => invoke('plan_cancel')

/**
 * Check what removing `pkgs` would break, and park the flow that would do it (DATA-FLOW §5).
 *
 * Call it **again** with `report.withDependents` for *Remove dependents too*. That option is not a
 * variant of the flow — it is this call repeated over a wider set, so a dependent of a dependent
 * surfaces on the next pass rather than being removed unannounced. The previous pass's flow is
 * discarded, as starting a new plan discards a previous preview.
 */
export const uninstallGuard = (env: PyEnv, pkgs: string[]): Promise<GuardReport> =>
  invoke('uninstall_guard', { env, pkgs })

/**
 * Snapshot, then remove. Streams `plan-progress`, and summarises like any other plan.
 *
 * `force` is §5's *Force remove only X*. Without it a removal the guard objected to rejects with
 * `PD-RES-004` before the snapshot is written, so a plan that will not run leaves nothing behind.
 */
export const uninstallExecute = (force: boolean): Promise<ExecutionOutcome> =>
  invoke('uninstall_execute', { force })

/**
 * Snapshots for an environment, newest first.
 *
 * Keyed by `envHash`, not by interpreter: snapshots outlive the Python that made them, so an
 * environment whose interpreter is gone still has a history worth showing.
 */
export const snapshotList = (envHash: string): Promise<SnapshotMeta[]> =>
  invoke('snapshot_list', { envHash })

/** Take a snapshot on demand, outside any plan. */
export const snapshotCreate = (env: PyEnv): Promise<SnapshotMeta> =>
  invoke('snapshot_create', { env })

/** The environment as it is now, against a snapshot. Claims no session. */
export const snapshotDiff = (env: PyEnv, id: string): Promise<Diff> =>
  invoke('snapshot_diff', { env, id })

/**
 * What restoring a snapshot would do, parking the flow that would do it.
 *
 * Split from `snapshotRollback` the way `planResolve` is split from `planExecute`: what the user
 * confirms has to be the plan they were shown, not one re-derived after they answered.
 */
export const snapshotRollbackPreview = (env: PyEnv, id: string): Promise<RollbackPreview> =>
  invoke('snapshot_rollback_preview', { env, id })

/**
 * Snapshot the current state, then restore the parked target. Streams `plan-progress`.
 *
 * Always returns a snapshot: the pre-rollback one, which is what makes a rollback itself
 * reversible — and why `latest` moves twice across a single restore.
 */
export const snapshotRollback = (): Promise<ExecutionOutcome> => invoke('snapshot_rollback')

/** What `reportBugUrl` returns — ERROR-CATALOG §4.3 splits the two deliberately. */
export interface BugReportLink {
  /** Prefilled GitHub issue URL, carrying a truncated tail-biased excerpt. */
  url: string
  /** The complete buffer, for the clipboard. Empty when nothing has run yet. */
  log: string
}

/**
 * Build the bug-report deep link. **Nothing is sent** — this returns a string.
 *
 * The URL carries a truncated excerpt because GitHub rejects very long ones; the full log comes
 * back separately so the UI can put it on the clipboard and say it did (§4.3).
 */
export const reportBugUrl = (env?: PyEnv, code?: string): Promise<BugReportLink> =>
  invoke('report_bug_url', { env: env ?? null, code: code ?? null })

/**
 * Version and availability for **both** engines.
 *
 * Both, because Settings shows a version beside each radio — asking about the configured one would
 * leave the other blank until it was picked. Takes a `PyEnv` because `Engine::info` does: pip's
 * version comes from `<python> -m pip --version`, so there is no env-free answer.
 */
export const engineInfo = (env: PyEnv): Promise<EngineInfo[]> => invoke('engine_info', { env })

/**
 * Upgrade pip inside `env` (PRD P0-10).
 *
 * Runs pip whatever engine is configured — upgrading pip is a pip operation by definition
 * (DATA-FLOW §7, amended by P1).
 *
 * Returns a `StepResult` carrying no versions: `from` and `to` are always absent, because the
 * adapter runs one command and never reads a version. The caller re-probes the environment to
 * refresh the row, which it has to do anyway. **No snapshot is taken** — DATA-FLOW §9.2's
 * exemption, because a snapshot restored by pip is no use when pip is what is broken.
 */
export const pipUpgrade = (env: PyEnv): Promise<StepResult> => invoke('pip_upgrade', { env })

/**
 * Run Code Health over a project folder (PRD P0-11).
 *
 * Streams `health-progress`, and on a fresh install syncs the tools venv first — so the first run
 * is the ~15 s bootstrap plus the tools, and the progress total already accounts for both.
 *
 * **A tool that fails does not fail the run.** It lands in `report.problems` and the others still
 * report, which is what `PD-HLT-003`'s "partial report" means. An empty `deptry` array is only
 * "clean" when `deptry` is in `report.ran`.
 */
export const healthRun = (env: PyEnv, project: string): Promise<HealthReport> =>
  invoke('health_run', { env, project })

/**
 * Subscribe to execution progress. Returns the unlisten function.
 *
 * Every step emits one `stepStarted`, any number of `line`s, and one `stepFinished` — which is
 * what lets the console drawer group by section and the live region count completions.
 */
export const onPlanProgress = (handler: (event: ProgressEvent) => void): Promise<UnlistenFn> =>
  listen<ProgressEvent>('plan-progress', (event) => {
    handler(event.payload)
  })

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

/**
 * Subscribe to Code Health progress. Returns the unlisten function.
 *
 * The same `ProgressEvent` payload `plan-progress` carries, on its own channel — ARCHITECTURE §7
 * lists `health-progress` as a channel, not a payload, and CLI-SPEC §6 already requires `health`
 * to emit the same shape. A second event type would cost a schema, a golden and a second console
 * renderer for nothing.
 */
export const onHealthProgress = (
  handler: (event: ProgressEvent) => void,
): Promise<UnlistenFn> =>
  listen<ProgressEvent>('health-progress', (event) => {
    handler(event.payload)
  })
