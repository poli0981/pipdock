/**
 * Settings' one control with teeth — SECURITY §3's PEP 668 override.
 *
 * Until 1.4.0 it was a checkbox that saved on click and that nothing read, so how easily it turned
 * on did not matter. Now that it does something, turning it on asks first with Cancel focused
 * (UI-SPEC §7), and turning it off — back to the protected default — does not.
 */

import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import * as ipc from '@/ipc'
import type { EnvRow, Settings } from '@/ipc'
import { PdSettings } from '@/screens/PdSettings'
import { useEnvStore, useSettingsStore } from '@/stores'
import { resetStore } from '@/test/stores'

vi.mock('@/ipc', async () => {
  const { ipcMock } = await import('@/test/ipc')
  return ipcMock()
})

const STORED: Settings = {
  engine: 'pip',
  locale: null,
  allowExternallyManaged: false,
  pinSuggestThreshold: 5,
}

/** Render with `stored` on disk, and wait until the fieldsets stop being disabled. */
async function setup(stored: Partial<Settings> = {}) {
  vi.mocked(ipc.settingsGet).mockResolvedValue({ ...STORED, ...stored })
  vi.mocked(ipc.settingsSet).mockImplementation((s: Settings) => Promise.resolve(s))
  render(<PdSettings />)
  const box = await screen.findByRole('checkbox', { name: /PEP 668 marks some Pythons/ })
  await waitFor(() => {
    expect(box).toBeEnabled()
  })
  return box
}

describe('the PEP 668 override', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    resetStore(useSettingsStore)
  })

  it('asks before turning on, with Cancel focused', async () => {
    const box = await setup()
    fireEvent.click(box)

    const dialog = screen.getByRole('dialog')
    expect(within(dialog).getByRole('button', { name: 'Cancel' })).toHaveFocus()
    expect(ipc.settingsSet).not.toHaveBeenCalled()
  })

  it('stays off when the question is cancelled', async () => {
    const box = await setup()
    fireEvent.click(box)
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Cancel' }))

    expect(screen.queryByRole('dialog')).toBeNull()
    expect(ipc.settingsSet).not.toHaveBeenCalled()
    expect(box).not.toBeChecked()
  })

  it('turns on once the user allows it, and only then', async () => {
    const box = await setup()
    fireEvent.click(box)
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Allow' }))

    await waitFor(() => {
      expect(box).toBeChecked()
    })
    expect(ipc.settingsSet).toHaveBeenCalledTimes(1)
    expect(ipc.settingsSet).toHaveBeenCalledWith(
      expect.objectContaining({ allowExternallyManaged: true }),
    )
  })

  it('turns off without asking', async () => {
    const box = await setup({ allowExternallyManaged: true })
    expect(box).toBeChecked()
    fireEvent.click(box)

    expect(screen.queryByRole('dialog')).toBeNull()
    await waitFor(() => {
      expect(box).not.toBeChecked()
    })
    expect(ipc.settingsSet).toHaveBeenCalledWith(
      expect.objectContaining({ allowExternallyManaged: false }),
    )
  })
})

describe('the engine radios', () => {
  const ROW: EnvRow = {
    interpreter: 'C:/work/.venv/Scripts/python.exe',
    source: 'venv-scan',
    envHash: 'hash-work',
    env: {
      interpreter: 'C:/work/.venv/Scripts/python.exe',
      prefix: 'C:/work/.venv',
      pythonVersion: '3.12.4',
      externallyManaged: false,
      hiddenUserSite: null,
      source: 'venv-scan',
    },
  }

  beforeEach(() => {
    vi.clearAllMocks()
    resetStore(useSettingsStore)
    resetStore(useEnvStore)
  })

  it('show each engine’s version for the selected environment, and say which is missing', async () => {
    useEnvStore.setState({ rows: [ROW], selected: ROW.interpreter })
    vi.mocked(ipc.engineInfo).mockResolvedValue([
      { id: 'pip', available: true, version: '26.2.1' },
      { id: 'uv', available: false },
    ])
    await setup()

    expect(await screen.findByText('26.2.1')).toBeInTheDocument()
    expect(screen.getByText('not found')).toBeInTheDocument()
    expect(ipc.engineInfo).toHaveBeenCalledWith(ROW.env)
  })

  it('ask nothing without an environment to ask about', async () => {
    await setup()
    expect(ipc.engineInfo).not.toHaveBeenCalled()
  })
})
