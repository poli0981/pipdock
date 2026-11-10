import { fireEvent, render, screen } from '@testing-library/react'
import { useState } from 'react'
import { describe, expect, it } from 'vitest'

import { PdConsoleDrawer } from '@/components/PdConsoleDrawer'

/** The drawer with the button that opens it, which is what `PdPlanPanel` renders around it. */
function Harness() {
  const [open, setOpen] = useState(false)
  return (
    <>
      <PdConsoleDrawer
        lines={[{ step: 0, pkg: 'numpy', stream: 'stdout', text: 'Collecting numpy' }]}
        open={open}
        done={1}
        total={2}
        onClose={() => {
          setOpen(false)
        }}
      />
      <button
        type="button"
        onClick={() => {
          setOpen(!open)
        }}
      >
        toggle
      </button>
    </>
  )
}

function openDrawer() {
  render(<Harness />)
  const toggle = screen.getByText('toggle')
  toggle.focus()
  fireEvent.click(toggle)
  return toggle
}

describe('PdConsoleDrawer', () => {
  it('names its close button for what it closes', () => {
    // It was labelled "Cancel", beside a *Stop* that cancels the run. One of the two was lying.
    openDrawer()
    expect(screen.getByRole('button', { name: 'Close console' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Cancel' })).toBeNull()
  })

  it('gives focus back to what opened it when Escape closes it', () => {
    // UI-SPEC §8: "Esc closes drawers". It did, and focus fell to <body>, so the next Tab started
    // from the top of the window.
    const toggle = openDrawer()
    const close = screen.getByRole('button', { name: 'Close console' })
    close.focus()

    fireEvent.keyDown(close, { key: 'Escape' })

    expect(screen.queryByRole('button', { name: 'Close console' })).toBeNull()
    expect(toggle).toHaveFocus()
  })

  it('gives focus back when its own button closes it', () => {
    const toggle = openDrawer()
    const close = screen.getByRole('button', { name: 'Close console' })
    close.focus()

    fireEvent.click(close)

    expect(toggle).toHaveFocus()
  })

  it('leaves focus alone when it was never inside', () => {
    // Toggled shut from the button that opened it: focus is already where it should be.
    const toggle = openDrawer()
    fireEvent.click(toggle)
    expect(toggle).toHaveFocus()
  })
})
