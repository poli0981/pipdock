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
  healthSaveReport: ReturnType<typeof vi.fn>
  healthFix: ReturnType<typeof vi.fn>
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
    healthSaveReport: vi.fn().mockResolvedValue([]),
    healthFix: vi.fn(),
    // Real, not a spy: the stores decide whether a rejection carries a catalog code by asking it,
    // and a stub that always answered one way would make every error test agree with itself.
    isPdError: (value: unknown): boolean =>
      typeof value === 'object' && value !== null && 'code' in value && 'message' in value,
  }
}
