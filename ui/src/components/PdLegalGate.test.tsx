/**
 * The legal gate, and the review mode that must not have changed it.
 *
 * This component had no test until Phase 4 added a second way to reach it. The first-run path is
 * the one that is legally load-bearing — a checkbox that gates Accept, and a Decline that closes
 * the window — so the cases below pin it rather than the new mode: the point of a presentational
 * prop is that the old rendering is untouched, and that is only true if something says so.
 */

import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { LEGAL_DOCUMENTS } from '@/components/legal'
import { PdLegalGate } from '@/components/PdLegalGate'
import { useLegalStore } from '@/stores'
import { resetStore } from '@/test/stores'

vi.mock('@/ipc', async () => {
  const { ipcMock } = await import('@/test/ipc')
  return ipcMock()
})
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn().mockResolvedValue(undefined) }))

const close = vi.fn().mockResolvedValue(undefined)
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ close }) }))

const { openUrl } = await import('@tauri-apps/plugin-opener')

describe('PdLegalGate', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    resetStore(useLegalStore)
    vi.mocked(openUrl).mockResolvedValue(undefined)
  })

  describe('first run', () => {
    it('lists all five documents and opens each on GitHub', () => {
      render(<PdLegalGate />)
      for (const doc of LEGAL_DOCUMENTS) {
        fireEvent.click(screen.getByRole('button', { name: labelFor(doc.key) }))
        expect(openUrl).toHaveBeenCalledWith(doc.href)
      }
      expect(openUrl).toHaveBeenCalledTimes(5)
    })

    it('keeps Accept disabled until the checkbox is ticked', () => {
      render(<PdLegalGate />)
      const accept = screen.getByRole('button', { name: 'Continue' })
      expect(accept).toBeDisabled()
      fireEvent.click(screen.getByRole('checkbox'))
      expect(accept).toBeEnabled()
    })

    it('closes the window on Decline', () => {
      render(<PdLegalGate />)
      fireEvent.click(screen.getByRole('button', { name: 'Decline and exit' }))
      expect(close).toHaveBeenCalledTimes(1)
    })

    it('says so when a document will not open', async () => {
      vi.mocked(openUrl).mockRejectedValueOnce(new Error('ForbiddenUrl'))
      render(<PdLegalGate />)
      fireEvent.click(screen.getByRole('button', { name: labelFor('license') }))
      // Never swallowed: this is the one screen whose whole purpose is showing the documents.
      expect(await screen.findByRole('alert')).toBeInTheDocument()
    })
  })

  describe('review mode', () => {
    it('offers Close instead of Accept and Decline, and cannot exit the app', () => {
      const onClose = vi.fn()
      render(<PdLegalGate review onClose={onClose} />)

      expect(screen.queryByRole('checkbox')).toBeNull()
      expect(screen.queryByRole('button', { name: 'Continue' })).toBeNull()
      expect(screen.queryByRole('button', { name: 'Decline and exit' })).toBeNull()

      fireEvent.click(screen.getByRole('button', { name: 'Close' }))
      expect(onClose).toHaveBeenCalledTimes(1)
      expect(close).not.toHaveBeenCalled()
    })

    it('still lists the same five documents', () => {
      render(<PdLegalGate review onClose={vi.fn()} />)
      for (const doc of LEGAL_DOCUMENTS) {
        expect(screen.getByRole('button', { name: labelFor(doc.key) })).toBeInTheDocument()
      }
    })
  })
})

/** The shipped English label for a document key — `setup.ts` loads the real catalogs. */
function labelFor(key: string): string {
  const labels: Record<string, string> = {
    license: 'License (GPL-3.0)',
    eula: 'End-User Licence Agreement',
    disclaimer: 'Disclaimer',
    privacy: 'Privacy Policy',
    thirdParty: 'Third-Party Notices',
  }
  return labels[key] ?? key
}
