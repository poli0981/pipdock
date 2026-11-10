import { fireEvent, render, screen, within } from '@testing-library/react'
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

  it('keeps a button focusable after it has been pressed', () => {
    // **Found in the installed build, not here.** *Add* and *Download the index* disabled
    // themselves the moment they were pressed, and a browser takes focus away from a control that
    // becomes `disabled` — so pressing either dropped focus to <body>. jsdom does not do that,
    // which is why no test could see it; what a test can hold is the contract that avoids it:
    // busy is `aria-disabled`, which greys the button and keeps it where focus is.
    useIndexStore.setState({
      query: 'num',
      hits: HITS,
      queue: [{ name: 'numpy', version: '' }],
      unavailable: 'missing',
      refreshing: true,
    })
    render(<PdSearch />)

    const queued = screen.getByRole('button', { name: 'Queued' })
    expect(queued).toHaveAttribute('aria-disabled', 'true')
    expect(queued).not.toBeDisabled()

    const refresh = screen.getByRole('button', { name: 'Downloading…' })
    expect(refresh).toHaveAttribute('aria-disabled', 'true')
    expect(refresh).not.toBeDisabled()
  })

  it('does nothing when a busy button is pressed again', () => {
    const refreshIndex = vi.fn()
    const enqueue = vi.fn()
    useIndexStore.setState({
      query: 'num',
      hits: HITS,
      queue: [{ name: 'numpy', version: '' }],
      unavailable: 'missing',
      refreshing: true,
      refreshIndex,
      enqueue,
    })
    render(<PdSearch />)

    fireEvent.click(screen.getByRole('button', { name: 'Queued' }))
    fireEvent.click(screen.getByRole('button', { name: 'Downloading…' }))

    expect(enqueue).not.toHaveBeenCalled()
    expect(refreshIndex).not.toHaveBeenCalled()
  })

  it('marks the result whose details are showing', () => {
    // Selection was a background colour and nothing else, which forced-colors erases.
    useIndexStore.setState({ query: 'num', hits: HITS, selected: 'numpydoc' })
    render(<PdSearch />)

    expect(screen.getByRole('button', { name: 'numpydoc' })).toHaveAttribute('aria-current', 'true')
    expect(screen.getByRole('button', { name: 'numpy' })).not.toHaveAttribute('aria-current')
  })
})
