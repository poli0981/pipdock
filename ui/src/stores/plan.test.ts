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
import type { FlowStep, GuardReport, Intent, PyEnv, RollbackPreview } from '@/ipc'
import { PANEL_PHASES, usePlanStore } from '@/stores/plan'
import flowStep from '@/test/fixtures/flow_step.json'
import rollbackPreview from '@/test/fixtures/rollback_preview.json'
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

/** Computed from the real `FlowStep` by `cargo run -p xtask -- ipc-fixtures`. */
const DECISIONS = flowStep as FlowStep
const SKIPPED: FlowStep = { step: 'nothing', reason: 'everything-skipped' }
const PREVIEW = rollbackPreview as RollbackPreview
const UPDATE_ALL: Intent = { intent: 'update', all: true, pkgs: [], except: [], forceLatest: false }

/** A promise the test settles by hand, so a second action can overtake the first. */
function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

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

  it('shows a resolved plan as a preview of the kind that was asked for', async () => {
    vi.mocked(ipc.planResolve).mockResolvedValue(DECISIONS)
    await usePlanStore.getState().resolve(ENV, { intent: 'install', specs: ['httpx'] })

    const { phase, step, kind, env } = usePlanStore.getState()
    expect(phase).toBe('preview')
    expect(step).toEqual(DECISIONS)
    expect(kind).toBe('install')
    expect(env).toEqual(ENV)
  })

  it('sends the round its answers belong to, and clears them with it', async () => {
    vi.mocked(ipc.planResolve).mockResolvedValue(DECISIONS)
    vi.mocked(ipc.planDecide).mockResolvedValue(SKIPPED)
    await usePlanStore.getState().resolve(ENV, UPDATE_ALL)
    usePlanStore.getState().choose('numpy', 'skip')
    usePlanStore.getState().choose('scipy', 'force-latest')

    await usePlanStore.getState().submitDecisions()

    expect(ipc.planDecide).toHaveBeenCalledWith({ numpy: 'skip', scipy: 'force-latest' })
    const { phase, step, decisions } = usePlanStore.getState()
    expect(phase).toBe('preview')
    expect(step).toEqual(SKIPPED)
    // The next preview may name other packages; an answer about one no longer in conflict
    // would be sent back to a flow that has moved on.
    expect(decisions).toEqual({})
  })

  it('keeps the preview usable when a round of decisions fails', async () => {
    // U24: a failed re-resolve leaves the flow as it was, so the screen must too — the same
    // preview, with the error beside it, decidable again.
    vi.mocked(ipc.planResolve).mockResolvedValue(DECISIONS)
    vi.mocked(ipc.planDecide).mockRejectedValue({ code: 'PD-NET-001', message: 'index down' })
    await usePlanStore.getState().resolve(ENV, UPDATE_ALL)
    usePlanStore.getState().choose('numpy', 'force-latest')

    await usePlanStore.getState().submitDecisions()

    const { phase, step, error, decisions } = usePlanStore.getState()
    expect(phase).toBe('preview')
    expect(step).toEqual(DECISIONS)
    expect(error?.code).toBe('PD-NET-001')
    expect(decisions).toEqual({ numpy: 'force-latest' })
  })

  it('previews a rollback as a rollback, and confirms it with the rollback command', async () => {
    vi.mocked(ipc.snapshotRollbackPreview).mockResolvedValue(PREVIEW)
    await usePlanStore.getState().rollback(ENV, PREVIEW.target.id)

    expect(ipc.snapshotRollbackPreview).toHaveBeenCalledWith(ENV, PREVIEW.target.id)
    const { phase, preview, kind } = usePlanStore.getState()
    expect(phase).toBe('preview')
    expect(preview).toEqual(PREVIEW)
    expect(kind).toBe('rollback')

    // One Confirm button for both flows: which command it sends is the session's kind.
    vi.mocked(ipc.snapshotRollback).mockRejectedValue({ code: 'PD-SNP-001', message: 'no room' })
    await usePlanStore.getState().execute()
    expect(ipc.snapshotRollback).toHaveBeenCalledOnce()
    expect(ipc.planExecute).not.toHaveBeenCalled()
  })

  it('lands a failed rollback preview where the user can see it', async () => {
    vi.mocked(ipc.snapshotRollbackPreview).mockRejectedValue({
      code: 'PD-SNP-002',
      message: 'that snapshot is gone',
    })
    await usePlanStore.getState().rollback(ENV, 'missing')

    expect(usePlanStore.getState().phase).toBe('failed')
    expect(usePlanStore.getState().error?.code).toBe('PD-SNP-002')
  })

  it('drops the answer to a resolve the user has already moved on from', async () => {
    // Every action bumps the generation; a reply from the one before it must not overwrite the
    // screen the user is now on — here, a preview arriving over an uninstall guard.
    const late = deferred<FlowStep>()
    vi.mocked(ipc.planResolve).mockReturnValue(late.promise)
    vi.mocked(ipc.uninstallGuard).mockResolvedValue(BREAKING)

    const resolving = usePlanStore.getState().resolve(ENV, UPDATE_ALL)
    await usePlanStore.getState().startUninstall(ENV, ['numpy'])
    late.resolve(DECISIONS)
    await resolving

    const { phase, step, guard } = usePlanStore.getState()
    expect(phase).toBe('guard')
    expect(guard).toEqual(BREAKING)
    expect(step).toBeNull()
  })

  it('drops a late failure the same way, for every action that waits', async () => {
    const lateResolve = deferred<FlowStep>()
    const lateDecide = deferred<FlowStep>()
    const latePreview = deferred<RollbackPreview>()
    vi.mocked(ipc.planResolve).mockResolvedValueOnce(DECISIONS).mockReturnValueOnce(lateResolve.promise)
    vi.mocked(ipc.planDecide).mockReturnValue(lateDecide.promise)
    vi.mocked(ipc.snapshotRollbackPreview).mockReturnValue(latePreview.promise)
    vi.mocked(ipc.uninstallGuard).mockResolvedValue(CLEAR)

    await usePlanStore.getState().resolve(ENV, UPDATE_ALL)
    const deciding = usePlanStore.getState().submitDecisions()
    const resolving = usePlanStore.getState().resolve(ENV, UPDATE_ALL)
    const previewing = usePlanStore.getState().rollback(ENV, 'id')
    await usePlanStore.getState().startUninstall(ENV, ['numpy', 'pandas'])

    lateDecide.reject({ code: 'PD-NET-001', message: 'late' })
    lateResolve.reject({ code: 'PD-NET-001', message: 'late' })
    latePreview.reject({ code: 'PD-NET-001', message: 'late' })
    await Promise.all([deciding, resolving, previewing])

    const { phase, error, guard } = usePlanStore.getState()
    expect(phase).toBe('guard')
    expect(guard).toEqual(CLEAR)
    expect(error).toBeNull()
  })
})
