/**
 * `PdPreviewDiff` grouping and `PdConflictRow`'s 3-way state — two of TESTING §2's five L3
 * obligations. (`PdSummarySheet` counts is the third, next door.)
 *
 * Fed from `flow_step.json`, serialized from the real `FlowStep` by
 * `cargo run -p xtask -- ipc-fixtures` and held current by a Rust-side staleness test. The
 * scenario covers all four `ChangeKind` variants plus a held-back and an impossible package, so
 * each rule below has a real subject rather than one invented for it.
 */

import { fireEvent, render, screen, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { PdPreviewDiff } from '@/components/PdPreviewDiff'
import type { Decision, FlowStep } from '@/ipc'
import flowStep from '@/test/fixtures/flow_step.json'

const STEP = flowStep as FlowStep
if (!('report' in STEP)) throw new Error('the fixture must be a step carrying a report')
const REPORT = STEP.report

function setup(decisions: Record<string, Decision> = {}) {
  const onChoose = vi.fn()
  render(<PdPreviewDiff report={REPORT} decisions={decisions} onChoose={onChoose} />)
  return { onChoose }
}

const section = (kind: string) => {
  const el = document.querySelector(`[data-section="${kind}"]`)
  if (el === null) throw new Error(`no section rendered for ${kind}`)
  return el as HTMLElement
}

describe('grouping', () => {
  it('puts a downgrade in its own section, not among the upgrades', () => {
    // The variant UI-SPEC had no home for. `2.0 → 1.9` under "Will upgrade" would be misleading
    // about exactly the change most likely to surprise.
    setup()
    expect(within(section('downgrade')).getByText('urllib3')).toBeInTheDocument()
    expect(within(section('upgrade')).queryByText('urllib3')).toBeNull()
    // And it reads as a downgrade, so the row itself is not ambiguous either.
    expect(within(section('downgrade')).getByText('2.2.0 → 1.26.20')).toBeInTheDocument()
  })

  it('separates what the user asked for from what came along with it', () => {
    setup()
    expect(within(section('new-install')).getByText('httpx')).toBeInTheDocument()
    expect(within(section('new-dependency')).getByText('tzdata')).toBeInTheDocument()
  })

  it('shows the version movement, never a bare target', () => {
    setup()
    expect(within(section('upgrade')).getByText('2.1.4 → 2.3.0')).toBeInTheDocument()
    // A new install has nothing to move from, so it is the one case that shows one version.
    expect(within(section('new-install')).getByText('0.28.1')).toBeInTheDocument()
  })

  it('counts each section in its own heading', () => {
    setup()
    expect(screen.getByText('Will upgrade (2)')).toBeInTheDocument()
    expect(screen.getByText('Will downgrade (1)')).toBeInTheDocument()
  })
})

describe('the 3-way conflict control', () => {
  const rowFor = (pkg: string) => {
    const el = document.querySelector(`[data-pkg="${pkg}"]`)
    if (el === null) throw new Error(`no conflict row for ${pkg}`)
    return el as HTMLElement
  }

  it('defaults a held-back package to Keep compatible', () => {
    // Mirrors `default_decision(is_impossible = false, …)`, and it is what makes "update
    // everything, one conflict kept compatible" still 4 clicks — the default costs none.
    setup()
    expect(
      within(rowFor('numpy')).getByRole('radio', { name: 'Keep compatible' }),
    ).toHaveAttribute('aria-checked', 'true')
  })

  it('disables Keep compatible on an impossible row and defaults it to Skip', () => {
    // `default_decision(is_impossible = true, …)` returns Skip, because there is no compatible
    // version to keep. Offering the choice would promise something core refuses to honour.
    setup()
    const row = rowFor('oldlib')
    expect(within(row).getByRole('radio', { name: 'Keep compatible' })).toBeDisabled()
    expect(within(row).getByRole('radio', { name: 'Skip' })).toHaveAttribute(
      'aria-checked',
      'true',
    )
  })

  it('names the blocker and its constraint, not just that it is stuck', () => {
    // PRD G2: conflicts are explained in one sentence a user can act on.
    setup()
    expect(
      within(rowFor('numpy')).getByText('scipy 1.11.4 requires numpy<1.28.0,>=1.21.6'),
    ).toBeInTheDocument()
  })

  it('does not force until the confirm is accepted', () => {
    // DISCLAIMER §2 and UI-SPEC §7: the one control that knowingly breaks a declared requirement
    // confirms first, and the safe option is the default.
    const { onChoose } = setup()
    fireEvent.click(within(rowFor('numpy')).getByRole('radio', { name: 'Force latest' }))
    expect(onChoose).not.toHaveBeenCalled()

    const dialog = within(rowFor('numpy')).getByRole('alertdialog')
    // Both dependents that exclude 2.5.1, from the computed fixture — so this also covers
    // the plural branch of `forceWarning`, which nothing else does.
    expect(within(dialog).getByText(/breaks pandas, scipy/)).toBeInTheDocument()

    fireEvent.click(within(dialog).getByRole('button', { name: 'Force latest' }))
    expect(onChoose).toHaveBeenCalledWith('numpy', 'force-latest')
  })

  it('abandons the force when the confirm is cancelled', () => {
    const { onChoose } = setup()
    fireEvent.click(within(rowFor('numpy')).getByRole('radio', { name: 'Force latest' }))
    const dialog = within(rowFor('numpy')).getByRole('alertdialog')
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }))
    expect(onChoose).not.toHaveBeenCalled()
    expect(within(rowFor('numpy')).queryByRole('alertdialog')).toBeNull()
  })

  it('reports Skip immediately, since nothing breaks by skipping', () => {
    const { onChoose } = setup()
    fireEvent.click(within(rowFor('numpy')).getByRole('radio', { name: 'Skip' }))
    expect(onChoose).toHaveBeenCalledWith('numpy', 'skip')
  })
})
