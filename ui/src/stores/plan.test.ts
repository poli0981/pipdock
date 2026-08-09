/**
 * `usePlanStore` — the repo's first store test, and the first use of the `@/ipc` mock.
 *
 * The store deliberately does not re-implement DATA-FLOW's state machine; the flow lives in Rust
 * and is resumable. What *is* here, and what these assert, is which command each transition sends
 * and which phase the screen is left in — the two things a component test cannot see and a Rust
 * test cannot either.
 */

import { beforeEach, describe, expect, it, vi } from 'vitest'

import * as ipc from '@/ipc'
import type { GuardReport, PyEnv } from '@/ipc'
import { PANEL_PHASES, usePlanStore } from '@/stores/plan'
import { resetStore } from '@/test/stores'

// The factory imports lazily because `vi.mock` is hoisted above every import in this file — a
// top-level `ipcMock` reference would be read before it is initialized.
vi.mock('@/ipc', async () => {
  const { ipcMock } = await import('@/test/ipc')
  return ipcMock()
})

const ENV: PyEnv = {
  interpreter: 'C:\\venv\\Scripts\\python.exe',
  prefix: 'C:\\venv',
  pythonVersion: '3.12.4',
  externallyManaged: false,
  hiddenUserSite: null,
  source: 'manual',
}

const BREAKING: GuardReport = {
  removing: ['numpy'],
  breaks: { numpy: [{ pkg: 'pandas', version: '2.1.4', constraint: '<2,>=1.26.0' }] },
  withDependents: ['numpy', 'pandas'],
}
const CLEAR: GuardReport = { removing: ['numpy', 'pandas'], breaks: {}, withDependents: [] }

describe('usePlanStore', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    resetStore(usePlanStore)
  })

  it('opens the guard without touching the screen behind it', async () => {
    vi.mocked(ipc.uninstallGuard).mockResolvedValue(BREAKING)
    await usePlanStore.getState().startUninstall(ENV, ['numpy'])

    const { phase, guard, kind } = usePlanStore.getState()
    expect(phase).toBe('guard')
    expect(kind).toBe('uninstall')
    expect(guard).toEqual(BREAKING)
    // `guard` is not a panel phase: the dialog opens *over* the table the user selected from, so
    // they can still see what they picked while deciding.
    expect(PANEL_PHASES.has(phase)).toBe(false)
  })

  it('re-guards the widened set instead of removing it', async () => {
    // The slice's exit criterion, at the layer that decides it. A dependent of a dependent has to
    // surface on the next pass, so widening is another `uninstall_guard` and never an execute.
    vi.mocked(ipc.uninstallGuard).mockResolvedValueOnce(BREAKING).mockResolvedValueOnce(CLEAR)
    await usePlanStore.getState().startUninstall(ENV, ['numpy'])
    await usePlanStore.getState().widen()

    expect(ipc.uninstallGuard).toHaveBeenCalledTimes(2)
    expect(ipc.uninstallGuard).toHaveBeenLastCalledWith(ENV, ['numpy', 'pandas'])
    expect(ipc.uninstallExecute).not.toHaveBeenCalled()
    expect(usePlanStore.getState().phase).toBe('guard')
    expect(usePlanStore.getState().guard).toEqual(CLEAR)
  })

  it('carries the force acknowledgement through to the command', async () => {
    vi.mocked(ipc.uninstallGuard).mockResolvedValue(BREAKING)
    vi.mocked(ipc.uninstallExecute).mockResolvedValue({
      summary: {
        planId: 'uninstall-abc',
        phase: 'isolated',
        results: [],
        check: { ok: true, findings: [] },
        counts: { ok: 1, failed: 0, skipped: 0 },
        cancelled: false,
      },
    })

    await usePlanStore.getState().startUninstall(ENV, ['numpy'])
    await usePlanStore.getState().confirmUninstall(true)

    expect(ipc.uninstallExecute).toHaveBeenCalledWith(true)
    expect(usePlanStore.getState().phase).toBe('summary')
  })

  it('lands a failed resolve somewhere the user can see it', async () => {
    // The S3/S4 hole: this used to set `idle`, which un-mounts the only thing that renders
    // `error`. The phase has to stay inside PANEL_PHASES or the failure is invisible.
    vi.mocked(ipc.planResolve).mockRejectedValue({
      code: 'PD-RES-003',
      message: 'a plan is already resolving or executing',
    })

    await usePlanStore.getState().resolve(ENV, {
      intent: 'update',
      all: true,
      pkgs: [],
      except: [],
      forceLatest: false,
    })

    const { phase, error } = usePlanStore.getState()
    expect(phase).toBe('failed')
    expect(PANEL_PHASES.has(phase)).toBe(true)
    expect(error?.code).toBe('PD-RES-003')
  })

  it('leaves a failed guard visible too', async () => {
    vi.mocked(ipc.uninstallGuard).mockRejectedValue({
      code: 'PD-ENV-002',
      message: 'this Python is externally managed (PEP 668)',
    })
    await usePlanStore.getState().startUninstall(ENV, ['numpy'])

    expect(usePlanStore.getState().phase).toBe('failed')
    expect(usePlanStore.getState().error?.code).toBe('PD-ENV-002')
  })

  it('subscribes to progress before the command runs, and unsubscribes after', async () => {
    // Order matters: subscribing after `uninstall_execute` starts drops the first steps into
    // nothing, and the console drawer opens on a run that is already part-way through.
    const order: string[] = []
    const unlisten = vi.fn()
    vi.mocked(ipc.onPlanProgress).mockImplementation(() => {
      order.push('subscribe')
      return Promise.resolve(unlisten)
    })
    vi.mocked(ipc.uninstallGuard).mockResolvedValue(CLEAR)
    vi.mocked(ipc.uninstallExecute).mockImplementation(() => {
      order.push('execute')
      // A `PdError` is a plain object on the wire, not an `Error` — which is exactly why the
      // stores ask `isPdError` rather than instanceof.
      // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors
      return Promise.reject({ code: 'PD-PRM-002', message: 'file locked' })
    })

    await usePlanStore.getState().startUninstall(ENV, ['numpy'])
    await usePlanStore.getState().confirmUninstall(false)

    expect(order).toEqual(['subscribe', 'execute'])
    // Even on failure: the summary still arrives, and a subscription left behind would double
    // every line of the next run.
    expect(unlisten).toHaveBeenCalledOnce()
    expect(usePlanStore.getState().phase).toBe('summary')
  })
})
