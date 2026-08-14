/**
 * The `@/ipc` mock, as TESTING §2 asks for: "IPC mocked at the typed wrapper layer".
 *
 * A **factory each test file passes to its own `vi.mock`**, not a module that registers the mock
 * as a side effect. `vi.mock` is hoisted to the top of the file being transformed and only that
 * file, so a side-effecting module would work or not depending on where the lint's import sort put
 * it — correctness by luck. Each file writes one line:
 *
 * ```ts
 * vi.mock('@/ipc', async () => {
 *   const { ipcMock } = await import('@/test/ipc')
 *   return ipcMock()
 * })
 * ```
 *
 * The factory imports lazily because the hoist puts `vi.mock` above every import in the file, so a
 * top-level reference to `ipcMock` is read before it is initialized.
 *
 * `importOriginal` is deliberately **not** used, and that is a trade: `isPdError` has to be real,
 * because both stores route every rejection through it, so it is re-exported here as the genuine
 * predicate rather than a stub. Everything else is a spy. Pulling the real module in instead would
 * drag `@tauri-apps/api` into jsdom, where `invoke` has no host to talk to.
 */

import { vi } from 'vitest'

/** The wrappers a test can drive. Add one when a test needs it, not before. */
export interface IpcMock {
  pkgList: ReturnType<typeof vi.fn>
  pkgOutdated: ReturnType<typeof vi.fn>
  pinList: ReturnType<typeof vi.fn>
  pinAdd: ReturnType<typeof vi.fn>
  pinRemove: ReturnType<typeof vi.fn>
  pinSuggestions: ReturnType<typeof vi.fn>
  envExport: ReturnType<typeof vi.fn>
  requirementsRead: ReturnType<typeof vi.fn>
  cacheUsage: ReturnType<typeof vi.fn>
  cacheClear: ReturnType<typeof vi.fn>
  planResolve: ReturnType<typeof vi.fn>
  planDecide: ReturnType<typeof vi.fn>
  planExecute: ReturnType<typeof vi.fn>
  planCancel: ReturnType<typeof vi.fn>
  uninstallGuard: ReturnType<typeof vi.fn>
  uninstallExecute: ReturnType<typeof vi.fn>
  onPlanProgress: ReturnType<typeof vi.fn>
  envScan: ReturnType<typeof vi.fn>
  envProbe: ReturnType<typeof vi.fn>
  pipUpgrade: ReturnType<typeof vi.fn>
  healthRun: ReturnType<typeof vi.fn>
  onHealthProgress: ReturnType<typeof vi.fn>
  pickProjectFolder: ReturnType<typeof vi.fn>
  pickSavePath: ReturnType<typeof vi.fn>
  pickOpenFile: ReturnType<typeof vi.fn>
  healthSaveReport: ReturnType<typeof vi.fn>
  healthFix: ReturnType<typeof vi.fn>
  healthDirty: ReturnType<typeof vi.fn>
  reportBugUrl: ReturnType<typeof vi.fn>
  appInfo: ReturnType<typeof vi.fn>
  isPdError: (value: unknown) => boolean
}

/**
 * Build the module object. Call it inside `vi.mock('@/ipc', () => ipcMock())`.
 *
 * Every command resolves to `undefined` unless the test says otherwise, which is what makes a test
 * that forgot to script one fail on the assertion rather than on a rejection from somewhere else.
 */
export function ipcMock(): IpcMock {
  return {
    pkgList: vi.fn().mockResolvedValue([]),
    pkgOutdated: vi.fn().mockResolvedValue([]),
    pinList: vi.fn().mockResolvedValue([]),
    pinAdd: vi.fn().mockResolvedValue(undefined),
    pinRemove: vi.fn().mockResolvedValue(true),
    // Empty by default: a screen that renders a suggestion section must not get one for free, or
    // a test asserting the section is absent passes for the wrong reason.
    pinSuggestions: vi.fn().mockResolvedValue([]),
    envExport: vi.fn().mockResolvedValue('C:/out/requirements.txt'),
    requirementsRead: vi.fn().mockResolvedValue({ specs: [], skipped: [] }),
    cacheUsage: vi.fn().mockResolvedValue({
      root: 'C:/data',
      database: { bytes: 0, path: 'C:/data/index.db', exists: false },
      snapshots: { bytes: 0, path: 'C:/data/snapshots', exists: false },
      tools: { bytes: 0, path: 'C:/data/tools', exists: false },
      snapshotCount: 0,
    }),
    cacheClear: vi.fn().mockResolvedValue(0),
    planResolve: vi.fn(),
    planDecide: vi.fn(),
    planExecute: vi.fn(),
    planCancel: vi.fn().mockResolvedValue(true),
    uninstallGuard: vi.fn(),
    uninstallExecute: vi.fn(),
    // Returns the unlisten function the store stores and calls in its `finally`.
    onPlanProgress: vi.fn().mockResolvedValue(() => undefined),
    envScan: vi.fn().mockResolvedValue([]),
    envProbe: vi.fn(),
    pipUpgrade: vi.fn().mockResolvedValue(undefined),
    healthRun: vi.fn(),
    onHealthProgress: vi.fn().mockResolvedValue(() => undefined),
    // Cancelled by default. A picker that silently returned a folder would let a test assert a
    // run it never actually asked for.
    pickProjectFolder: vi.fn().mockResolvedValue(null),
    pickSavePath: vi.fn().mockResolvedValue(null),
    pickOpenFile: vi.fn().mockResolvedValue(null),
    healthSaveReport: vi.fn().mockResolvedValue([]),
    healthFix: vi.fn(),
    healthDirty: vi.fn().mockResolvedValue(null),
    // `PdErrorRow` asks for this on mount, so any test that renders a screen containing an error
    // needs it — a gap that only appears once a *screen* is under test rather than the row alone.
    //
    // It must be the whole `BugReportLink`. This resolved a bare string for two milestones, so a
    // test that clicked *Copy full log* was asserting against `writeText(undefined)` and agreeing
    // with itself. A mock of the wrong *shape* is worse than no mock: it makes the seam it covers
    // look tested.
    reportBugUrl: vi.fn().mockResolvedValue({
      url: 'https://github.com/poli0981/pipdock/issues/new',
      log: '[log excerpt]',
    }),
    // The one command in the surface that cannot fail — `app_info` is a `const fn` in Rust
    // returning `AppInfo`, not `Wire<AppInfo>`. So it resolves by default rather than being a bare
    // `vi.fn()`: About renders version and docs hash unconditionally and has no error branch, and
    // a mock resolving `undefined` would invent one.
    appInfo: vi.fn().mockResolvedValue({ version: '0.1.0', docsHash: 'a1b2c3d4e5f60718' }),
    // Real, not a spy: the stores decide whether a rejection carries a catalog code by asking it,
    // and a stub that always answered one way would make every error test agree with itself.
    isPdError: (value: unknown): boolean =>
      typeof value === 'object' && value !== null && 'code' in value && 'message' in value,
  }
}
