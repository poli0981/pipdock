/**
 * What *Report bug* puts in the issue — U31.
 *
 * The row asked `report_bug_url` for its link with no environment, so every prefilled issue said
 * nothing about the Python or the engine version — the first two questions anyone answering it
 * would ask. Passing the environment costs a subprocess (`pip --version`), so it is asked for when
 * the user reports, not once per row mounted: a summary of a failed batch can mount dozens.
 */

import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { PdErrorRow } from '@/components/PdErrorRow'
import * as ipc from '@/ipc'
import type { EnvRow, PyEnv } from '@/ipc'
import { useEnvStore } from '@/stores'
import { resetStore } from '@/test/stores'

vi.mock('@/ipc', async () => {
  const { ipcMock } = await import('@/test/ipc')
  return ipcMock()
})
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn().mockResolvedValue(undefined) }))
const { openUrl } = await import('@tauri-apps/plugin-opener')

const ENV: PyEnv = {
  interpreter: 'C:/work/.venv/Scripts/python.exe',
  prefix: 'C:/work/.venv',
  pythonVersion: '3.12.4',
  externallyManaged: false,
  hiddenUserSite: null,
  source: 'venv-scan',
}

const ROW: EnvRow = {
  interpreter: ENV.interpreter,
  source: 'venv-scan',
  envHash: 'hash-work',
  env: ENV,
}

describe('Report bug', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    resetStore(useEnvStore)
    useEnvStore.setState({ rows: [ROW], selected: ENV.interpreter })
    vi.mocked(ipc.reportBugUrl).mockResolvedValue({
      url: 'https://github.com/poli0981/pipdock/issues/new',
      log: '[log excerpt]',
    })
  })

  it('does not spawn anything to render the row', async () => {
    render(<PdErrorRow error={{ code: 'PD-NET-001', message: 'offline' }} />)
    await screen.findByText('Report bug')
    // One call, without the environment: deciding whether there is a log to offer is free.
    expect(ipc.reportBugUrl).toHaveBeenCalledTimes(1)
    expect(ipc.reportBugUrl).toHaveBeenCalledWith(undefined, 'PD-NET-001')
  })

  it('asks again with the environment when the user reports', async () => {
    render(<PdErrorRow error={{ code: 'PD-NET-001', message: 'offline' }} />)
    fireEvent.click(await screen.findByText('Report bug'))

    await waitFor(() => {
      expect(openUrl).toHaveBeenCalledTimes(1)
    })
    expect(ipc.reportBugUrl).toHaveBeenLastCalledWith(ENV, 'PD-NET-001')
  })
})
