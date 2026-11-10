/**
 * The held-back row — UI-SPEC §4's three-way control, and the blocker sentence under it.
 *
 * **Why this file exists.** For two releases the preview rendered `scipy requires scipy 1.11.4
 * requires numpy <1.28.0,>=1.21.6`: `graph::blockers_for` built a whole English sentence into
 * `Blocker.constraint` and this component wrapped it in `plan.blocker` a second time. Neither
 * suite could see it. `PdPreviewDiff.test.tsx` asserted the *right* sentence against a fixture
 * whose blockers were hand-written to the shape `Blocker`'s own doc describes — a shape core had
 * stopped producing — and this component had no test of its own at all. The fixture is computed
 * from the real graph now (`fixtures::numpy_blockers`), so the two cannot drift apart again.
 *
 * The blockers here therefore come from that fixture rather than from literals, for the reason
 * `PdUninstallDialog.test.tsx` gives about `GuardReport`.
 */

import { fireEvent, render, screen, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { PdConflictRow } from '@/components/PdConflictRow'
import type { Blocker, FlowStep } from '@/ipc'
import flowStep from '@/test/fixtures/flow_step.json'

const STEP = flowStep as FlowStep
if (!('report' in STEP)) throw new Error('the fixture must be a step carrying a report')

/** The computed blockers holding `numpy` back: `pandas 2.1.4` and `scipy 1.11.4`. */
const BLOCKERS = STEP.report.heldBack?.[0]?.blockers ?? []

function setup(blockers: readonly Blocker[], impossible = false) {
  const onChoose = vi.fn()
  render(
    <ul>
      <PdConflictRow
        pkg="numpy"
        resolved="1.26.4"
        latest="2.5.1"
        blockers={blockers}
        impossible={impossible}
        value={undefined}
        onChoose={onChoose}
      />
    </ul>,
  )
  return { onChoose }
}

describe('PdConflictRow', () => {
  it('names each blocker once, with its version and its constraint', () => {
    // The defect this file was written for. `getByText` is exact by default, so a doubled
    // dependent name fails here rather than merely looking wrong.
    setup(BLOCKERS)

    expect(screen.getByText('pandas 2.1.4 requires numpy<2,>=1.26.0')).toBeInTheDocument()
    expect(screen.getByText('scipy 1.11.4 requires numpy<1.28.0,>=1.21.6')).toBeInTheDocument()
    expect(screen.queryByText(/requires.*requires/)).not.toBeInTheDocument()
  })

  it('drops the version clause when the probe did not know one', () => {
    setup([{ by: 'scipy', constraint: 'numpy<1.28.0,>=1.21.6' }])

    expect(screen.getByText('scipy requires numpy<1.28.0,>=1.21.6')).toBeInTheDocument()
  })

  it('shows an unattributed constraint verbatim rather than inventing a culprit', () => {
    // ARCHITECTURE §3: if attribution is ambiguous, show the constraint without a culprit. This
    // is also the shape `PdPreviewDiff` synthesizes for an impossible row.
    setup([{ constraint: 'no version of oldlib is compatible with python 3.12' }], true)

    expect(
      screen.getByText('no version of oldlib is compatible with python 3.12'),
    ).toBeInTheDocument()
  })

  it('is one tab stop, with the arrows moving between its choices', () => {
    // U47: three buttons with `role="radio"` and none of a radio group's keyboard — every choice
    // of every conflict was its own tab stop, and the arrow keys did nothing.
    const { onChoose } = setup(BLOCKERS)
    const radios = within(screen.getByRole('radiogroup', { name: 'numpy' })).getAllByRole('radio')
    const [keep, skip, force] = radios
    if (keep === undefined || skip === undefined || force === undefined) {
      throw new Error('three choices expected')
    }

    // The checked one is the stop: keep-compatible is the default.
    expect(radios.map((r) => r.getAttribute('tabindex'))).toEqual(['0', '-1', '-1'])

    keep.focus()
    fireEvent.keyDown(keep, { key: 'ArrowRight' })
    expect(skip).toHaveFocus()
    fireEvent.keyDown(skip, { key: 'ArrowDown' })
    expect(force).toHaveFocus()
    fireEvent.keyDown(force, { key: 'ArrowRight' })
    expect(keep).toHaveFocus()
    fireEvent.keyDown(keep, { key: 'ArrowLeft' })
    expect(force).toHaveFocus()

    // **Focus moves; nothing is chosen.** A radio group normally checks what the arrows land on,
    // and here that would open *Force latest*'s destructive confirm from an arrow key.
    expect(onChoose).not.toHaveBeenCalled()
    expect(screen.queryByRole('alertdialog')).toBeNull()
  })

  it('skips a choice that cannot be made', () => {
    setup(BLOCKERS, true)
    const [, skip, force] = within(screen.getByRole('radiogroup', { name: 'numpy' })).getAllByRole(
      'radio',
    )
    if (skip === undefined || force === undefined) throw new Error('three choices expected')

    force.focus()
    fireEvent.keyDown(force, { key: 'ArrowRight' })
    expect(skip).toHaveFocus()
  })

  it('refuses Keep compatible on an impossible row', () => {
    // `default_decision(is_impossible = true, …)` returns Skip, so the control mirrors the core
    // rather than offering a choice it would refuse to honour.
    setup(BLOCKERS, true)

    const group = screen.getByRole('radiogroup', { name: 'numpy' })
    expect(within(group).getByRole('radio', { name: /keep compatible/i })).toBeDisabled()
  })
})
