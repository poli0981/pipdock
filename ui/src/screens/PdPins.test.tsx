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

const INTERPRETER = 'C:\\venv\\Scripts\\python.exe'

/**
 * Put the store where it would be after an environment was selected and loaded.
 *
 * The row carries `env`, because a real one does whenever the probe succeeded — and
 * `pin_suggestions` takes a whole `PyEnv`, so a fixture without it silently tests the
 * probe-failed path instead. `env` being optional is the point of the last case below.
 */
function withEnv(pins: Pin[] = PINS) {
  useEnvStore.setState({
    selected: INTERPRETER,
    loadedFor: INTERPRETER,
    rows: [
      {
        interpreter: INTERPRETER,
        envHash: 'envhash01',
        source: 'manual',
        env: { interpreter: INTERPRETER, pythonVersion: '3.12.4', externallyManaged: false },
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

  describe('suggestions', () => {
    /** Two suggestions, as `pin_suggestions` orders them: most-depended-upon first. */
    const SUGGESTIONS = [
      { pkg: 'urllib3', dependents: 12 },
      { pkg: 'certifi', dependents: 1 },
    ]

    it('asks for suggestions once per environment, not once per render', async () => {
      // Each call is a `probe.py` run. StrictMode double-invokes effects and the store's guard is
      // checked synchronously for that reason — the same defect `loadSnapshots` records, and the
      // same one `app_info` shipped with.
      withEnv()
      vi.mocked(ipc.pinSuggestions).mockResolvedValue(SUGGESTIONS)
      const first = render(<PdPins />)
      await vi.waitFor(() => {
        expect(ipc.pinSuggestions).toHaveBeenCalledTimes(1)
      })
      first.unmount()
      render(<PdPins />)
      await vi.waitFor(() => {
        expect(screen.getByText(/urllib3 — 12 packages depend on it/)).toBeInTheDocument()
      })
      expect(ipc.pinSuggestions).toHaveBeenCalledTimes(1)
    })

    it('states the count, and pluralises it', async () => {
      // UI-SPEC §4 fixes this sentence. One dependent is a different sentence from twelve, and a
      // catalog with only `_other` renders "1 packages" — which is the failure I18N §1 forbids.
      withEnv()
      vi.mocked(ipc.pinSuggestions).mockResolvedValue(SUGGESTIONS)
      render(<PdPins />)

      expect(
        await screen.findByText('urllib3 — 12 packages depend on it.'),
      ).toBeInTheDocument()
      expect(screen.getByText('certifi — 1 package depends on it.')).toBeInTheDocument()
    })

    it('pins with the count as the reason, and drops the suggestion', async () => {
      // PRD P1-2: "suggest pin with reason". The reason is editable below, so this is a starting
      // point rather than the last word — but a pin with no reason is the mystery the reason
      // field exists to prevent.
      withEnv([])
      vi.mocked(ipc.pinSuggestions).mockResolvedValue(SUGGESTIONS)
      render(<PdPins />)

      fireEvent.click(await screen.findByLabelText('Pin urllib3'))
      await vi.waitFor(() => {
        expect(ipc.pinAdd).toHaveBeenCalledWith('envhash01', {
          pkg: 'urllib3',
          mode: 'exclude',
          reason: '12 packages depended on it',
        })
      })
      // Gone from the section without a second probe: the answer cannot have changed.
      await vi.waitFor(() => {
        expect(screen.queryByLabelText('Pin urllib3')).not.toBeInTheDocument()
      })
      expect(ipc.pinSuggestions).toHaveBeenCalledTimes(1)
    })

    it('caps the list and says how many it did not show', async () => {
      // Found by running against the 352-package fixture: the default threshold of 5 qualifies 94
      // packages there. A quarter of the environment is not a suggestion, it is a second package
      // list — and the deep trees where that happens are exactly where the feature matters, so
      // the answer is a bounded view, not a higher default.
      withEnv()
      vi.mocked(ipc.pinSuggestions).mockResolvedValue(
        Array.from({ length: 94 }, (_, i) => ({ pkg: `pkg${i}`, dependents: 100 - i })),
      )
      render(<PdPins />)

      await vi.waitFor(() => {
        expect(screen.getByText(/86 more packages would qualify/)).toBeInTheDocument()
      })
      // The count is from the *full* list, so a capped view never misreports the total.
      expect(screen.getAllByRole('button', { name: /^Pin pkg/ })).toHaveLength(8)
    })

    it('renders nothing at all when there is nothing to suggest', () => {
      // An empty "Worth pinning" heading is a question nobody asked.
      withEnv()
      render(<PdPins />)
      expect(screen.queryByText('Worth pinning')).not.toBeInTheDocument()
    })

    it('stays quiet when the suggestion fetch fails', async () => {
      // Advisory. This screen's job is listing pins, and it does that either way — so a failure
      // must not put an error row above the thing the user came for.
      withEnv()
      vi.mocked(ipc.pinSuggestions).mockRejectedValue({ code: 'PD-ENV-003', message: 'nope' })
      render(<PdPins />)

      await vi.waitFor(() => {
        expect(ipc.pinSuggestions).toHaveBeenCalled()
      })
      expect(screen.queryByText('Worth pinning')).not.toBeInTheDocument()
      expect(screen.queryByRole('alert')).not.toBeInTheDocument()
      // The list it exists to show is still there.
      expect(document.querySelector('[data-pin="numpy"]')).not.toBeNull()
    })

    it('asks for nothing without an environment', () => {
      render(<PdPins />)
      expect(ipc.pinSuggestions).not.toHaveBeenCalled()
    })

    it('asks for nothing when the environment could not be probed', () => {
      // `EnvRow.env` is absent exactly when the probe failed, and a suggestion needs an
      // interpreter to probe. The pin list still renders — it is keyed by hash and outlives the
      // Python that made it.
      useEnvStore.setState({
        selected: INTERPRETER,
        loadedFor: INTERPRETER,
        rows: [{ interpreter: INTERPRETER, envHash: 'envhash01', source: 'manual' } as never],
        pins: PINS,
      })
      render(<PdPins />)

      expect(ipc.pinSuggestions).not.toHaveBeenCalled()
      expect(document.querySelector('[data-pin="numpy"]')).not.toBeNull()
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
