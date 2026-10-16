import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import '@/i18n'
import * as ipc from '@/ipc'

import { PdLogs } from './PdLogs'

vi.mock('@/ipc', async () => {
  const { ipcMock } = await import('@/test/ipc')
  return ipcMock()
})

vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({
  writeText: vi.fn().mockResolvedValue(undefined),
}))

const { writeText } = await import('@tauri-apps/plugin-clipboard-manager')
const logsTail = vi.mocked(ipc.logsTail)

describe('PdLogs', () => {
  beforeEach(() => {
    logsTail.mockResolvedValue({ dir: 'C:/data/logs', lines: [] })
  })

  it('shows where the log is, as a path', async () => {
    render(<PdLogs />)
    expect(await screen.findByText('C:/data/logs')).toBeInTheDocument()
    expect(screen.getByText(/nothing has been logged yet/i)).toBeInTheDocument()
  })

  it('copies what the log says at the moment of copying', async () => {
    render(<PdLogs />)
    await screen.findByText('C:/data/logs')
    // The run the user is reporting happened after the screen opened.
    logsTail.mockResolvedValue({ dir: 'C:/data/logs', lines: ['spawn', 'exited code=Some(1)'] })
    fireEvent.click(screen.getByRole('button', { name: /copy recent log/i }))
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledWith('spawn\nexited code=Some(1)')
    })
    expect(await screen.findByText(/copied/i)).toBeInTheDocument()
  })
})
