/**
 * A control that is busy stays where focus is, and ignores a second press — UI-SPEC §8.
 *
 * A browser takes focus away from a control that becomes `disabled`. Nine buttons disabled
 * themselves the moment they were pressed, so the keypress that started a scan, a run or an audit
 * also dropped focus to `<body>`. jsdom does not do that, which is why this was found in the
 * installed build and not here. What jsdom *can* hold is the contract that replaced it: busy is
 * `aria-disabled`, the button is still focusable, and the click that `disabled` used to swallow
 * is swallowed by a guard instead.
 *
 * That guard is the half worth testing. `disabled` cannot be clicked; `aria-disabled` can, so a
 * guard that is missing or wrong starts a second scan over the first.
 */

import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { PdRequirements } from '@/components/PdRequirements'
import * as ipc from '@/ipc'
import type { EnvRow } from '@/ipc'
import { PdEnvironments } from '@/screens/PdEnvironments'
import { PdHealth } from '@/screens/PdHealth'
import { PdPlanPanel } from '@/screens/PdPlanPanel'
import { PdSecurity } from '@/screens/PdSecurity'
import { useEnvStore, useHealthStore, usePlanStore, useSecurityStore } from '@/stores'
import { resetStore } from '@/test/stores'

vi.mock('@/ipc', async () => {
  const actual = await vi.importActual<typeof import('@/ipc')>('@/ipc')
  const { ipcMock } = await import('@/test/ipc')
  return {
    ...actual,
    ...ipcMock(),
    onScanProgress: vi.fn().mockResolvedValue(() => undefined),
    onAuditProgress: vi.fn().mockResolvedValue(() => undefined),
  }
})

const row = (name: string): EnvRow => {
  const interpreter = `C:\\${name}\\.venv\\Scripts\\python.exe`
  return {
    interpreter,
    source: 'venv-scan',
    envHash: name,
    env: {
      interpreter,
      prefix: `C:\\${name}\\.venv`,
      pythonVersion: '3.12.10',
      externallyManaged: false,
      hiddenUserSite: null,
      source: 'venv-scan',
    },
  }
}
const A = row('a')
const B = row('b')

/** Busy, focusable, and not natively disabled. */
function expectBusy(button: HTMLElement) {
  expect(button).toHaveAttribute('aria-disabled', 'true')
  expect(button).not.toBeDisabled()
}

beforeEach(() => {
  vi.clearAllMocks()
  resetStore(useEnvStore)
  resetStore(useHealthStore)
  resetStore(useSecurityStore)
  resetStore(usePlanStore)
})

describe('Environments', () => {
  it('ignores Use on the environment already in use, and selects another', () => {
    const select = vi.fn()
    useEnvStore.setState({ rows: [A, B], selected: A.interpreter, scan: vi.fn(), select })
    render(<PdEnvironments />)
    const [useA, useB] = screen.getAllByRole('button', { name: 'Use' })
    if (useA === undefined || useB === undefined) throw new Error('two rows expected')

    expectBusy(useA)
    fireEvent.click(useA)
    expect(select).not.toHaveBeenCalled()

    fireEvent.click(useB)
    expect(select).toHaveBeenCalledWith(B.interpreter)
  })

  it('does not start a second scan over the first', () => {
    const scan = vi.fn()
    useEnvStore.setState({ rows: [A], selected: A.interpreter, scanning: true, scan })
    render(<PdEnvironments />)
    scan.mockClear() // the screen scans once on arrival

    const rescan = screen.getByRole('button', { name: 'Rescan' })
    expectBusy(rescan)
    fireEvent.click(rescan)
    expect(scan).not.toHaveBeenCalled()
  })

  it('rescans when asked and idle', () => {
    const scan = vi.fn()
    useEnvStore.setState({ rows: [A], selected: A.interpreter, scanning: false, scan })
    render(<PdEnvironments />)
    scan.mockClear()

    fireEvent.click(screen.getByRole('button', { name: 'Rescan' }))
    expect(scan).toHaveBeenCalledOnce()
  })
})

describe('Code Health', () => {
  it('does not start a second run, and starts one when idle', () => {
    const run = vi.fn()
    useEnvStore.setState({ rows: [A], selected: A.interpreter })
    useHealthStore.setState({ folder: 'C:/proj', phase: 'running', run })
    const { unmount } = render(<PdHealth />)

    const running = screen.getByRole('button', { name: 'Running…' })
    expectBusy(running)
    fireEvent.click(running)
    expect(run).not.toHaveBeenCalled()
    unmount()

    useHealthStore.setState({ phase: 'idle' })
    render(<PdHealth />)
    fireEvent.click(screen.getByRole('button', { name: 'Run' }))
    expect(run).toHaveBeenCalledWith(A.env, A.envHash)
  })

  it('is disabled outright when there is nothing to run against', () => {
    // Unavailable is still `disabled`: nobody is focused on a control that cannot be used.
    useEnvStore.setState({ rows: [A], selected: A.interpreter })
    useHealthStore.setState({ folder: null, phase: 'idle' })
    render(<PdHealth />)
    expect(screen.getByRole('button', { name: 'Run' })).toBeDisabled()
  })
})

describe('Security', () => {
  it('does not start a second audit, and starts one when idle', () => {
    const run = vi.fn()
    useEnvStore.setState({ rows: [A], selected: A.interpreter })
    useSecurityStore.setState({ phase: 'running', run })
    const { unmount } = render(<PdSecurity />)

    const busy = screen.getByRole('button', { name: 'Run audit' })
    expectBusy(busy)
    fireEvent.click(busy)
    expect(run).not.toHaveBeenCalled()
    unmount()

    useSecurityStore.setState({ phase: 'idle' })
    render(<PdSecurity />)
    fireEvent.click(screen.getByRole('button', { name: 'Run audit' }))
    expect(run).toHaveBeenCalledWith(A.env, A.envHash)
  })
})

describe('the plan panel', () => {
  it('asks to stop once', () => {
    const cancel = vi.fn()
    usePlanStore.setState({ phase: 'executing', kind: 'update', cancelling: false, cancel })
    const { unmount } = render(<PdPlanPanel onFinished={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }))
    expect(cancel).toHaveBeenCalledOnce()
    unmount()

    usePlanStore.setState({ cancelling: true })
    render(<PdPlanPanel onFinished={vi.fn()} />)
    const stopping = screen.getByRole('button', { name: 'Stopping…' })
    expectBusy(stopping)
    fireEvent.click(stopping)
    expect(cancel).toHaveBeenCalledOnce()
  })
})

describe('requirements', () => {
  it('does not open a second picker while an export is being written', async () => {
    vi.mocked(ipc.pickSavePath).mockResolvedValue('C:/out/requirements.txt')
    // Never settles: the export is in flight for as long as the test looks.
    vi.mocked(ipc.envExport).mockReturnValue(new Promise(() => undefined))
    render(<PdRequirements env={A.env} />)
    const section = within(screen.getByRole('region', { name: 'Requirements file' }))
    const exportButton = section.getByRole('button', { name: 'Export…' })

    fireEvent.click(exportButton)
    await waitFor(() => {
      expectBusy(exportButton)
    })
    fireEvent.click(exportButton)
    fireEvent.click(section.getByRole('button', { name: 'Import…' }))

    expect(ipc.pickSavePath).toHaveBeenCalledOnce()
    expect(ipc.pickOpenFile).not.toHaveBeenCalled()
  })

  it('does not open a second picker while a file is being read', async () => {
    vi.mocked(ipc.pickOpenFile).mockResolvedValue('C:/in/requirements.txt')
    vi.mocked(ipc.requirementsRead).mockReturnValue(new Promise(() => undefined))
    render(<PdRequirements env={A.env} />)
    const section = within(screen.getByRole('region', { name: 'Requirements file' }))
    const importButton = section.getByRole('button', { name: 'Import…' })

    fireEvent.click(importButton)
    await waitFor(() => {
      expectBusy(importButton)
    })
    fireEvent.click(importButton)

    expect(ipc.pickOpenFile).toHaveBeenCalledOnce()
  })

  it('offers neither without an environment', () => {
    render(<PdRequirements />)
    const section = within(screen.getByRole('region', { name: 'Requirements file' }))
    expect(section.getByRole('button', { name: 'Export…' })).toBeDisabled()
  })
})
