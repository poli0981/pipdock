/**
 * The Pins screen — the repo's first *screen* test.
 *
 * Leaf components take props; a screen reads a store and calls commands, so this is the first test
 * that has to mock `@/ipc` and reset a store between cases. The pin fixture is generated from the
 * real Rust types, so a field renamed in `pins.rs` fails here rather than drifting.
 */

import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import * as ipc from '@/ipc'
import type { Pin } from '@/ipc'
import { PdPins } from '@/screens/PdPins'
import { useEnvStore } from '@/stores'
import pinFixture from '@/test/fixtures/pin_list.json'
import { resetStore } from '@/test/stores'

vi.mock('@/ipc', async () => {
  const { ipcMock } = await import('@/test/ipc')
  return ipcMock()
})

const PINS = pinFixture as Pin[]

/** Put the store where it would be after an environment was selected and loaded. */
function withEnv(pins: Pin[] = PINS) {
  useEnvStore.setState({
    selected: 'C:\\venv\\Scripts\\python.exe',
    loadedFor: 'C:\\venv\\Scripts\\python.exe',
    rows: [
      {
        interpreter: 'C:\\venv\\Scripts\\python.exe',
        envHash: 'envhash01',
        source: 'manual',
      } as never,
    ],
    pins,
  })
}

describe('PdPins', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    resetStore(useEnvStore)
  })

  it('says there is no list, not an empty one, without an environment', () => {
    // Pins are keyed by `envHash`. "No pins" here would read as "this environment has none".
    render(<PdPins />)
    // `PdEmptyState` prefixes the mono glyph UI-SPEC §7 asks for, so this is a substring match.
    expect(screen.getByText(/no environment selected/)).toBeInTheDocument()
  })

  it('renders one row per pin, telling the two modes apart', () => {
    withEnv()
    render(<PdPins />)

    // The fixture holds a Hold on numpy and an Exclude on scipy — §4 says only "a 🔒 chip", but a
    // Hold restates a version in every plan and an Exclude does not.
    expect(document.querySelector('[data-pin="numpy"]')).not.toBeNull()
    expect(document.querySelector('[data-pin="scipy"]')).not.toBeNull()
    expect(screen.getByText('1.26.4')).toBeInTheDocument()
    expect(screen.getByText('pinned')).toBeInTheDocument()
  })

  it('shows an existing reason and commits an edit on blur', async () => {
    withEnv()
    render(<PdPins />)

    const field = screen.getByLabelText('Reason for pinning numpy')
    expect(field).toHaveValue('scipy 1.11.4 needs numpy < 1.28')

    fireEvent.change(field, { target: { value: '  pinned for the 2.x migration  ' } })
    // Not on change: one `pin_add` upsert per keystroke is a lot of SQLite for a text field.
    expect(ipc.pinAdd).not.toHaveBeenCalled()

    fireEvent.blur(field)
    await vi.waitFor(() => {
      expect(ipc.pinAdd).toHaveBeenCalledWith('envhash01', {
        pkg: 'numpy',
        // The mode is carried through untouched: changing what a pin *is* must not be a side
        // effect of typing in its reason box.
        mode: { hold: { version: '1.26.4' } },
        reason: 'pinned for the 2.x migration',
      })
    })
  })

  it('clears a reason as null rather than an empty string', async () => {
    withEnv()
    render(<PdPins />)

    const field = screen.getByLabelText('Reason for pinning numpy')
    fireEvent.change(field, { target: { value: '   ' } })
    fireEvent.blur(field)

    // `exactOptionalPropertyTypes` is on and `reason` is optional, so `''` would round-trip as a
    // reason that exists and says nothing.
    await vi.waitFor(() => {
      expect(ipc.pinAdd).toHaveBeenCalledWith('envhash01', {
        pkg: 'numpy',
        mode: { hold: { version: '1.26.4' } },
      })
    })
  })

  it('unpins through the same path the table uses', async () => {
    withEnv()
    render(<PdPins />)

    const row = document.querySelector('[data-pin="scipy"]')
    fireEvent.click(row?.querySelector('[data-action="unpin"]') as HTMLElement)
    await vi.waitFor(() => {
      expect(ipc.pinRemove).toHaveBeenCalledWith('envhash01', 'scipy')
    })
  })

  it('offers no way to create a Hold pin', () => {
    // `pins::hold_requirements` is dead code and `plan_requirements` restates the *installed*
    // version, so a hold at another version is a promise nothing keeps. The CLI cannot create one
    // either; the screen must not be the head that disagrees.
    withEnv()
    render(<PdPins />)
    expect(screen.queryByText(/hold/i)).not.toBeInTheDocument()
  })
})
