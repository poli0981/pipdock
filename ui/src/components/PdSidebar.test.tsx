import { render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { NAV_KEYS } from '@/components/nav'
import { PdSidebar } from '@/components/PdSidebar'
import { usePlanStore, useUiStore } from '@/stores'
import { resetStore } from '@/test/stores'

vi.mock('@/ipc', async () => {
  const { ipcMock } = await import('@/test/ipc')
  return ipcMock()
})

describe('PdSidebar', () => {
  beforeEach(() => {
    resetStore(usePlanStore)
    useUiStore.setState({ nav: 'installed' })
  })

  it('tells assistive technology which shortcut each tab has', () => {
    // The `^1`…`^9` hints are `aria-hidden`, so the shortcut was visible and nowhere else.
    render(<PdSidebar />)
    const tabs = screen.getAllByRole('button')

    expect(tabs).toHaveLength(NAV_KEYS.length)
    expect(tabs.map((tab) => tab.getAttribute('aria-keyshortcuts'))).toEqual(
      NAV_KEYS.map((_, i) => `Control+${String(i + 1)}`),
    )
  })

  it('draws the shortcut hints in a token, not at half opacity', () => {
    // `opacity-50` over `text-text-dim` measured about 2.6:1 on the sidebar's background. Being
    // hidden from a screen reader does not make visible text exempt from contrast.
    render(<PdSidebar />)
    for (const hint of screen.getAllByText(/^\^\d$/)) {
      expect(hint.className).not.toMatch(/opacity-/)
      expect(hint).toHaveClass('text-text-dim')
    }
  })

  it('marks the tab that is showing', () => {
    render(<PdSidebar />)
    expect(screen.getByRole('button', { current: 'page' })).toHaveTextContent('Installed')
  })
})
