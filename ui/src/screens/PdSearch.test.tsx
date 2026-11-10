import { render, screen, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { Hit } from '@/ipc'
import { PdSearch } from '@/screens/PdSearch'
import { useEnvStore, usePlanStore } from '@/stores'
import { useIndexStore } from '@/stores/index-search'
import { resetStore } from '@/test/stores'

vi.mock('@/ipc', async () => {
  const { ipcMock } = await import('@/test/ipc')
  return ipcMock()
})

const HITS: Hit[] = [
  { name: 'numpy', display: 'numpy', kind: 'exact' },
  { name: 'numpydoc', display: 'numpydoc', kind: 'prefix' },
]

describe('what Search says without being looked at', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    resetStore(useIndexStore)
    resetStore(useEnvStore)
    resetStore(usePlanStore)
  })

  it('keeps one status region on screen from the first render', () => {
    // A live region announces *changes* to itself. One that is mounted together with its first
    // message has no earlier state to differ from, and most readers say nothing.
    render(<PdSearch />)
    expect(screen.getByRole('status')).toBeInTheDocument()
    expect(screen.getByRole('status')).toBeEmptyDOMElement()
  })

  it('says it is searching', () => {
    useIndexStore.setState({ query: 'num', searching: true })
    render(<PdSearch />)
    expect(screen.getByRole('status')).toHaveTextContent('Searching…')
  })

  it('says the index is still loading', () => {
    useIndexStore.setState({ ready: false })
    render(<PdSearch />)
    expect(screen.getByRole('status')).toHaveTextContent('Preparing the package index…')
  })

  it('says how many results came back', () => {
    // The list simply appeared. Nothing told a screen-reader user that the keystroke had an
    // answer, or how long the answer was.
    useIndexStore.setState({ query: 'num', hits: HITS })
    render(<PdSearch />)
    expect(screen.getByRole('status')).toHaveTextContent('2 results')
  })

  it('says when nothing matched, in the region as well as in the empty state', () => {
    useIndexStore.setState({ query: 'zzzz', hits: [] })
    render(<PdSearch />)
    expect(within(screen.getByRole('status')).getByText(/nothing matches/)).toHaveClass('sr-only')
  })

  it('marks the result whose details are showing', () => {
    // Selection was a background colour and nothing else, which forced-colors erases.
    useIndexStore.setState({ query: 'num', hits: HITS, selected: 'numpydoc' })
    render(<PdSearch />)

    expect(screen.getByRole('button', { name: 'numpydoc' })).toHaveAttribute('aria-current', 'true')
    expect(screen.getByRole('button', { name: 'numpy' })).not.toHaveAttribute('aria-current')
  })
})
