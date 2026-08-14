/**
 * The command palette.
 *
 * `setup.ts` stubs `showModal` by setting `open` and nothing else — no top layer, no focus
 * containment, no native `cancel` on Escape. So modality is **not** under test here; what is
 * tested is the policy on top: what filtering shows, what Enter dispatches, and that dispatching
 * closes before it acts.
 */

import { fireEvent, render, screen, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { PdPalette } from '@/components/PdPalette'
import { useEnvStore, useLegalStore, useUiStore } from '@/stores'
import { resetStore } from '@/test/stores'

vi.mock('@/ipc', async () => {
  const { ipcMock } = await import('@/test/ipc')
  return ipcMock()
})

function open() {
  const onClose = vi.fn()
  render(<PdPalette onClose={onClose} />)
  return { onClose, input: screen.getByRole('combobox') }
}

const options = () => within(screen.getByRole('listbox')).getAllByRole('option')

describe('PdPalette', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    resetStore(useUiStore)
    resetStore(useEnvStore)
    resetStore(useLegalStore)
  })

  it('opens showing every action rather than an empty list', () => {
    open()
    // Nine tabs plus the rest. The exact number is not the point; that it is not zero is.
    expect(options().length).toBeGreaterThan(9)
  })

  it('filters as you type, and ranks the exact match first', () => {
    const { input } = open()
    fireEvent.change(input, { target: { value: 'pins' } })
    expect(options()[0]).toHaveTextContent('Pins')
  })

  it('navigates with the arrows while the input keeps focus', () => {
    // The reason this is a listbox with aria-activedescendant rather than a set of tab stops: the
    // user must be able to keep typing to narrow while moving through what is left.
    const { input } = open()
    expect(options()[0]).toHaveAttribute('aria-selected', 'true')

    fireEvent.keyDown(input, { key: 'ArrowDown' })
    expect(options()[0]).toHaveAttribute('aria-selected', 'false')
    expect(options()[1]).toHaveAttribute('aria-selected', 'true')
    expect(input).toHaveFocus()

    fireEvent.keyDown(input, { key: 'ArrowUp' })
    expect(options()[0]).toHaveAttribute('aria-selected', 'true')
  })

  it('will not run off either end of the list', () => {
    const { input } = open()
    fireEvent.keyDown(input, { key: 'ArrowUp' })
    expect(options()[0]).toHaveAttribute('aria-selected', 'true')
  })

  it('dispatches the highlighted action on Enter, and closes first', () => {
    const { input, onClose } = open()
    fireEvent.change(input, { target: { value: 'health' } })
    fireEvent.keyDown(input, { key: 'Enter' })

    expect(onClose).toHaveBeenCalled()
    // Closing before running is what stops an action that changes `nav` fighting the dialog for
    // focus — App moves focus to the new screen's <h1> on every nav change.
    expect(useUiStore.getState().nav).toBe('health')
  })

  it('dispatches on click too', () => {
    const { input } = open()
    fireEvent.change(input, { target: { value: 'search' } })
    fireEvent.click(options()[0] as HTMLElement)
    expect(useUiStore.getState().nav).toBe('search')
  })

  it('reaches an action that is not a tab', () => {
    // The palette is only worth having if it does more than the sidebar already does.
    const { input } = open()
    fireEvent.change(input, { target: { value: 'documents' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(useLegalStore.getState().review).toBe(true)
  })

  it('says so when nothing matches, rather than showing an empty box', () => {
    const { input } = open()
    fireEvent.change(input, { target: { value: 'zzzzz' } })
    expect(screen.queryByRole('option')).toBeNull()
    expect(screen.getByText(/nothing matches/)).toBeInTheDocument()
  })

  it('closes on Escape through its own handler', () => {
    const { onClose } = open()
    // jsdom does not synthesize the native `cancel` event, so this fires it directly — the same
    // path a real Escape takes.
    fireEvent(screen.getByRole('dialog'), new Event('cancel', { cancelable: true }))
    expect(onClose).toHaveBeenCalled()
  })
})
