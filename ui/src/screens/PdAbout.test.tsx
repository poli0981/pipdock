/**
 * About — the screen, and the seam it opens into the legal gate.
 *
 * Two things here are worth more than they look. The five documents are asserted against the
 * *shared* `LEGAL_DOCUMENTS`, which is what makes extracting that module load-bearing rather than
 * tidy: consent is recorded against exactly those five, so a second list would eventually offer a
 * document the gate never showed. And the links page is asserted twice — once for the exact URL,
 * once for the host — because a constant edited past `capabilities/external-links.json` fails
 * *silently* at runtime, which is the failure mode SECURITY §4 exists to name.
 */

import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { LEGAL_DOCUMENTS, REPO } from '@/components/legal'
import * as ipc from '@/ipc'
import { PdAbout } from '@/screens/PdAbout'
import { resetAppInfoCache } from '@/screens/useAppInfo'
import { useLegalStore } from '@/stores'
import { resetStore } from '@/test/stores'

vi.mock('@/ipc', async () => {
  const { ipcMock } = await import('@/test/ipc')
  return ipcMock()
})
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn().mockResolvedValue(undefined) }))
vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({
  writeText: vi.fn().mockResolvedValue(undefined),
}))

const { openUrl } = await import('@tauri-apps/plugin-opener')
const { writeText } = await import('@tauri-apps/plugin-clipboard-manager')

describe('PdAbout', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    resetStore(useLegalStore)
    resetAppInfoCache()
    vi.mocked(openUrl).mockResolvedValue(undefined)
    vi.mocked(ipc.appInfo).mockResolvedValue({ version: '1.2.3', docsHash: 'abc123def456' })
  })

  it('renders an h1, which App focuses on every nav change', async () => {
    render(<PdAbout />)
    expect(await screen.findByRole('heading', { level: 1, name: 'About' })).toBeInTheDocument()
  })

  it('reads the version and hash from app_info, not from a literal', async () => {
    render(<PdAbout />)
    expect(await screen.findByText('1.2.3')).toBeInTheDocument()
    expect(screen.getByText('abc123def456')).toBeInTheDocument()
    // The SPDX identifier, not the looser "GPL-3.0" the README used to say.
    expect(screen.getByText('GPL-3.0-only')).toBeInTheDocument()
  })

  it('asks the bridge once however many times it is mounted', async () => {
    // `App.tsx` keys <main> on the nav key, so every visit to About is a fresh mount. `app_info`
    // is a `const fn` — the answer cannot change — so a second call is pure waste, and "every
    // screen fetching twice on mount" is a defect this repo has already shipped once.
    const first = render(<PdAbout />)
    expect(await screen.findByText('1.2.3')).toBeInTheDocument()
    first.unmount()
    render(<PdAbout />)
    expect(await screen.findByText('1.2.3')).toBeInTheDocument()
    await waitFor(() => {
      expect(ipc.appInfo).toHaveBeenCalledTimes(1)
    })
  })

  it('shows both addresses and copies them verbatim', async () => {
    render(<PdAbout />)
    for (const address of ['contact@poli0981.dev', 'code@poli0981.dev']) {
      // Visible, so the screen still works for someone with no mail client — which is also the
      // reason these are copied rather than opened with mailto:.
      expect(screen.getByText(address)).toBeInTheDocument()
      fireEvent.click(screen.getByLabelText(`Copy ${address}`))
      await waitFor(() => {
        expect(writeText).toHaveBeenCalledWith(address)
      })
    }
    expect(writeText).toHaveBeenCalledTimes(2)
  })

  it('opens the links page inside the capability scope', () => {
    render(<PdAbout />)
    fireEvent.click(screen.getByRole('button', { name: 'Open the links page' }))
    expect(openUrl).toHaveBeenCalledWith('https://poli0981.dev/links/')
    // Not redundant with the line above: this is the assertion that fails if someone edits the
    // constant to a host `external-links.json` does not allow, where the promise rejects and
    // nothing on screen would otherwise say so.
    const [href] = vi.mocked(openUrl).mock.calls[0] ?? []
    expect(String(href).startsWith('https://poli0981.dev/')).toBe(true)
  })

  it('opens the repository from the shared constant', () => {
    render(<PdAbout />)
    fireEvent.click(screen.getByRole('button', { name: 'Open the repository' }))
    expect(openUrl).toHaveBeenCalledWith(REPO)
    expect(REPO.startsWith('https://github.com/')).toBe(true)
  })

  it('says so when a link will not open', async () => {
    vi.mocked(openUrl).mockRejectedValueOnce(new Error('ForbiddenUrl'))
    render(<PdAbout />)
    fireEvent.click(screen.getByRole('button', { name: 'Open the links page' }))
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Could not open that link in your browser.',
    )
  })

  it('re-opens the documents without touching consent', () => {
    render(<PdAbout />)
    fireEvent.click(screen.getByRole('button', { name: 'Show the documents again' }))
    expect(useLegalStore.getState().review).toBe(true)
    // The whole point of `review` being its own flag: a review is not a revocation.
    expect(useLegalStore.getState().accepted).toBeNull()
  })

  it('offers the same five documents the gate records consent against', () => {
    expect(LEGAL_DOCUMENTS).toHaveLength(5)
    expect(LEGAL_DOCUMENTS.map((d) => d.key)).toEqual([
      'license',
      'eula',
      'disclaimer',
      'privacy',
      'thirdParty',
    ])
    for (const doc of LEGAL_DOCUMENTS) {
      expect(doc.href.startsWith('https://github.com/')).toBe(true)
    }
  })
})
