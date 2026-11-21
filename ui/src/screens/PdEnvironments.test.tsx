/**
 * *Add interpreter…* and *Remove from list* — PRD P0-1's manual browse (1.6.0).
 *
 * The store half and the screen half, because the seam between them is where this app's bugs have
 * lived (CLAUDE.md, `PdPlanPanel`). Both key on the interpreter **path**: the first version merged
 * and removed by `envHash`, which follows links — so a venv made with `--symlinks` merged into the
 * base Python it points at, and *Remove from list* took every row sharing the hash with it. An
 * independent review of 1.6.0 found both, and the cases below are the ones it named.
 */

import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import * as ipc from '@/ipc'
import type { EnvRow, PdError } from '@/ipc'
import { PdEnvironments } from '@/screens/PdEnvironments'
import { useEnvStore } from '@/stores'
import { resetStore } from '@/test/stores'

vi.mock('@/ipc', async () => {
  const actual = await vi.importActual<typeof import('@/ipc')>('@/ipc')
  const { ipcMock } = await import('@/test/ipc')
  return {
    ...actual,
    ...ipcMock(),
    onScanProgress: vi.fn().mockResolvedValue(() => undefined),
  }
})

const row = (name: string, source: EnvRow['source'] = 'venv-scan'): EnvRow => {
  const interpreter = `D:\\work\\${name}\\Scripts\\python.exe`
  return {
    interpreter,
    source,
    envHash: `hash-${name}`,
    env: {
      interpreter,
      prefix: `D:\\work\\${name}`,
      pythonVersion: '3.13.9',
      externallyManaged: false,
      hiddenUserSite: null,
      source,
    },
  }
}
const DISCOVERED = row('discovered', 'registry')
const ADDED = row('myenv', 'manual')

const missing: PdError = { code: 'PD-ENV-001', message: 'no interpreter at E:\\gone\\python.exe' }
const GONE: EnvRow = {
  interpreter: 'E:\\gone\\python.exe',
  source: 'manual',
  envHash: 'hash-gone',
  error: missing,
}

beforeEach(() => {
  vi.clearAllMocks()
  resetStore(useEnvStore)
})

describe('the store', () => {
  it('asks for a python.exe with the title it was given, and a cancelled picker changes nothing', async () => {
    vi.mocked(ipc.pickOpenFile).mockResolvedValueOnce(null)
    useEnvStore.setState({ rows: [DISCOVERED], selected: DISCOVERED.interpreter })

    await useEnvStore.getState().addManual('Choose one')

    expect(ipc.pickOpenFile).toHaveBeenCalledWith('Choose one', ['exe'])
    expect(ipc.envAddManual).not.toHaveBeenCalled()
    const state = useEnvStore.getState()
    expect(state.addingManual).toBe(false)
    expect(state.rows).toEqual([DISCOVERED])
    expect(state.selected).toBe(DISCOVERED.interpreter)
  })

  it('appends the new row, selects it and says so', async () => {
    vi.mocked(ipc.pickOpenFile).mockResolvedValueOnce('d:\\work\\myenv\\scripts\\python.exe')
    vi.mocked(ipc.envAddManual).mockResolvedValueOnce(ADDED)
    useEnvStore.setState({ rows: [DISCOVERED], selected: DISCOVERED.interpreter })

    await useEnvStore.getState().addManual('Choose one')

    expect(ipc.envAddManual).toHaveBeenCalledWith('d:\\work\\myenv\\scripts\\python.exe')
    const state = useEnvStore.getState()
    // The row Rust returned, with the canonical path it stored — not the spelling picked.
    expect(state.rows).toEqual([DISCOVERED, ADDED])
    expect(state.selected).toBe(ADDED.interpreter)
    expect(state.manualNotice).toEqual({ kind: 'added', path: ADDED.interpreter })
    expect(state.addingManual).toBe(false)
  })

  it('does not list an interpreter twice when discovery already has it', async () => {
    // The same file in another case — Windows paths are case-insensitive: one row, the
    // discovered one, under the spelling the next scan will list, selected.
    vi.mocked(ipc.pickOpenFile).mockResolvedValueOnce(DISCOVERED.interpreter)
    vi.mocked(ipc.envAddManual).mockResolvedValueOnce({
      ...ADDED,
      envHash: DISCOVERED.envHash,
      interpreter: DISCOVERED.interpreter.toUpperCase(),
    })
    useEnvStore.setState({ rows: [DISCOVERED], selected: null })

    await useEnvStore.getState().addManual('Choose one')

    const state = useEnvStore.getState()
    expect(state.rows).toEqual([DISCOVERED])
    expect(state.selected).toBe(DISCOVERED.interpreter)
  })

  it('lists a venv made with --symlinks beside the base Python it links to, and selects the venv', async () => {
    // Its `python.exe` is a link to the base, so the two share an `envHash`. Merging on the hash
    // selected the base: the next update would have changed the system's Python, not the venv's.
    const linked: EnvRow = { ...ADDED, envHash: DISCOVERED.envHash }
    vi.mocked(ipc.pickOpenFile).mockResolvedValueOnce(linked.interpreter)
    vi.mocked(ipc.envAddManual).mockResolvedValueOnce(linked)
    useEnvStore.setState({ rows: [DISCOVERED], selected: DISCOVERED.interpreter })

    await useEnvStore.getState().addManual('Choose one')

    const state = useEnvStore.getState()
    expect(state.rows).toEqual([DISCOVERED, linked])
    expect(state.selected).toBe(linked.interpreter)
  })

  it('replaces a broken row when its venv is made again, and selects the working one', async () => {
    const fresh: EnvRow = {
      interpreter: GONE.interpreter,
      source: 'manual',
      envHash: GONE.envHash,
      env: {
        interpreter: GONE.interpreter,
        prefix: 'E:\\gone',
        pythonVersion: '3.13.9',
        externallyManaged: false,
        hiddenUserSite: null,
        source: 'manual',
      },
    }
    vi.mocked(ipc.pickOpenFile).mockResolvedValueOnce(GONE.interpreter)
    vi.mocked(ipc.envAddManual).mockResolvedValueOnce(fresh)
    useEnvStore.setState({ rows: [DISCOVERED, GONE], selected: DISCOVERED.interpreter })

    await useEnvStore.getState().addManual('Choose one')

    const state = useEnvStore.getState()
    expect(state.rows).toEqual([DISCOVERED, fresh])
    expect(state.rows[1]?.error).toBeUndefined()
    expect(state.selected).toBe(GONE.interpreter)
  })

  it('takes the spelling Rust stored when an added interpreter is added again', async () => {
    // `add_manual_env` rewrites the stored text, so the next scan lists the new spelling; a row
    // kept under the old one would leave the selection naming a row that is about to be renamed.
    const respelled: EnvRow = { ...ADDED, interpreter: ADDED.interpreter.toLowerCase() }
    vi.mocked(ipc.pickOpenFile).mockResolvedValueOnce(respelled.interpreter)
    vi.mocked(ipc.envAddManual).mockResolvedValueOnce(respelled)
    useEnvStore.setState({ rows: [DISCOVERED, ADDED], selected: ADDED.interpreter })

    await useEnvStore.getState().addManual('Choose one')

    const state = useEnvStore.getState()
    expect(state.rows).toEqual([DISCOVERED, respelled])
    expect(state.selected).toBe(respelled.interpreter)
  })

  it('keeps a refused path off the list and shows why', async () => {
    const refused: PdError = {
      code: 'PD-ENV-001',
      message: 'D:\\notepad.exe is not a Python interpreter PipDock will run',
    }
    vi.mocked(ipc.pickOpenFile).mockResolvedValueOnce('D:\\notepad.exe')
    vi.mocked(ipc.envAddManual).mockRejectedValueOnce(refused)
    useEnvStore.setState({ rows: [DISCOVERED] })

    await useEnvStore.getState().addManual('Choose one')

    const state = useEnvStore.getState()
    expect(state.rows).toEqual([DISCOVERED])
    expect(state.error?.code).toBe('PD-ENV-001')
    expect(state.addingManual).toBe(false)
  })

  it('refuses to add while a scan runs, and to scan while an add runs', async () => {
    // A scan that read the list before the add wrote it would replace `rows` without the new row.
    useEnvStore.setState({ scanning: true })
    await useEnvStore.getState().addManual('Choose one')
    expect(ipc.pickOpenFile).not.toHaveBeenCalled()

    useEnvStore.setState({ scanning: false, addingManual: true })
    await useEnvStore.getState().scan()
    expect(ipc.envScan).not.toHaveBeenCalled()
  })

  it('removes a row, falls back from a selection it held and closes its detail', async () => {
    useEnvStore.setState({
      rows: [DISCOVERED, ADDED],
      selected: ADDED.interpreter,
      openFor: ADDED.interpreter,
    })

    await useEnvStore.getState().removeManual(ADDED.interpreter)

    // By the path the row shows: for a file that is gone, the hash Rust would recompute from the
    // stored text can differ from the one it was added under, and removing by it never matched.
    expect(ipc.envRemoveManual).toHaveBeenCalledWith(ADDED.interpreter)
    const state = useEnvStore.getState()
    expect(state.rows).toEqual([DISCOVERED])
    expect(state.selected).toBe(DISCOVERED.interpreter)
    expect(state.openFor).toBeNull()
    expect(state.manualNotice).toEqual({ kind: 'removed', path: ADDED.interpreter })
    expect(state.removingManual).toBeNull()
  })

  it('leaves the selection alone when another row is removed', async () => {
    useEnvStore.setState({ rows: [DISCOVERED, GONE], selected: DISCOVERED.interpreter })
    await useEnvStore.getState().removeManual(GONE.interpreter)
    expect(useEnvStore.getState().selected).toBe(DISCOVERED.interpreter)
    expect(useEnvStore.getState().rows).toEqual([DISCOVERED])
  })

  it('removes only the row it was asked about when another shares its envHash', async () => {
    const linked: EnvRow = { ...ADDED, envHash: DISCOVERED.envHash }
    useEnvStore.setState({ rows: [DISCOVERED, linked], selected: DISCOVERED.interpreter })
    await useEnvStore.getState().removeManual(linked.interpreter)
    expect(useEnvStore.getState().rows).toEqual([DISCOVERED])
  })

  it('refuses to remove while a scan runs, and to scan while a removal runs', async () => {
    // A scan that read the list before the removal was written would bring the row back.
    useEnvStore.setState({ rows: [DISCOVERED, ADDED], scanning: true })
    await useEnvStore.getState().removeManual(ADDED.interpreter)
    expect(ipc.envRemoveManual).not.toHaveBeenCalled()
    expect(useEnvStore.getState().rows).toEqual([DISCOVERED, ADDED])

    useEnvStore.setState({ scanning: false, removingManual: ADDED.interpreter })
    await useEnvStore.getState().scan()
    expect(ipc.envScan).not.toHaveBeenCalled()
  })
})

describe('the screen', () => {
  it('offers Add interpreter… beside Rescan, busy while a scan runs', () => {
    const addManual = vi.fn()
    useEnvStore.setState({ rows: [DISCOVERED], scanning: true, scan: vi.fn(), addManual })
    render(<PdEnvironments />)

    const add = screen.getByRole('button', { name: 'Add interpreter…' })
    expect(add).toHaveAttribute('aria-disabled', 'true')
    expect(add).not.toBeDisabled()
    fireEvent.click(add)
    expect(addManual).not.toHaveBeenCalled()

    act(() => {
      useEnvStore.setState({ scanning: false })
    })
    fireEvent.click(add)
    expect(addManual).toHaveBeenCalledWith('Choose a Python interpreter (python.exe)')
  })

  it('makes Add interpreter… the empty state’s one action', () => {
    useEnvStore.setState({ rows: [], scanning: false, scan: vi.fn() })
    render(<PdEnvironments />)
    expect(screen.getAllByRole('button', { name: 'Add interpreter…' })).toHaveLength(2)
  })

  it('offers Remove from list on added rows only, broken ones included', () => {
    useEnvStore.setState({ rows: [DISCOVERED, ADDED, GONE], scan: vi.fn() })
    render(<PdEnvironments />)

    const removes = screen.getAllByRole('button', { name: /^Remove from list: / })
    expect(removes.map((b) => b.getAttribute('aria-label'))).toEqual([
      `Remove from list: ${ADDED.interpreter}`,
      `Remove from list: ${GONE.interpreter}`,
    ])
    // Forgetting an entry deletes nothing, so it is not drawn as destructive (UI-SPEC §7).
    for (const button of removes) expect(button).not.toHaveAttribute('data-destructive')
  })

  it('makes Remove from list busy while a scan runs, without taking focus from it', () => {
    const removeManual = vi.fn()
    useEnvStore.setState({ rows: [ADDED], scanning: true, scan: vi.fn(), removeManual })
    render(<PdEnvironments />)

    const remove = screen.getByRole('button', { name: `Remove from list: ${ADDED.interpreter}` })
    expect(remove).toHaveAttribute('aria-disabled', 'true')
    expect(remove).not.toBeDisabled()
    fireEvent.click(remove)
    expect(removeManual).not.toHaveBeenCalled()

    act(() => {
      useEnvStore.setState({ scanning: false })
    })
    fireEvent.click(remove)
    expect(removeManual).toHaveBeenCalledWith(ADDED.interpreter)
  })

  it('says what was added in a status region that was there before the words', () => {
    useEnvStore.setState({ rows: [DISCOVERED], scan: vi.fn() })
    render(<PdEnvironments />)
    const status = screen.getByRole('status')
    expect(status).toHaveTextContent('')

    act(() => {
      useEnvStore.setState({
        rows: [DISCOVERED, ADDED],
        manualNotice: { kind: 'added', path: ADDED.interpreter },
      })
    })
    expect(screen.getByRole('status')).toBe(status)
    expect(status).toHaveTextContent(`Added ${ADDED.interpreter} to the list.`)
  })

  it('moves focus to the heading when the row whose button had it goes', async () => {
    useEnvStore.setState({ rows: [DISCOVERED, ADDED], selected: DISCOVERED.interpreter, scan: vi.fn() })
    render(<PdEnvironments />)
    const remove = screen.getByRole('button', { name: `Remove from list: ${ADDED.interpreter}` })
    remove.focus()
    expect(remove).toHaveFocus()

    fireEvent.click(remove)

    await waitFor(() => {
      expect(screen.queryByRole('button', { name: /^Remove from list: / })).toBeNull()
    })
    expect(screen.getByRole('heading', { level: 1, name: 'Environments' })).toHaveFocus()
  })
})
