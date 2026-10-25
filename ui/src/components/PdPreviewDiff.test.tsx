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
import type { Decision, FlowStep, ResolutionReport } from '@/ipc'
import flowStep from '@/test/fixtures/flow_step.json'
import flowStepForced from '@/test/fixtures/flow_step_forced.json'

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

describe('what the report says besides the changes', () => {
  // `flow_step_forced.json`: numpy forced past pandas and scipy, numba left out for its Python,
  // and a withdrawn release in the plan. `forced` and `incompatible` are computed by the functions
  // the flow calls, so these assert what the application would actually show. The report carried
  // all three from 1.4.0's wire change; nothing rendered any of them.
  const FORCED_STEP = flowStepForced as FlowStep
  if (!('report' in FORCED_STEP)) throw new Error('the forced fixture must carry a report')
  const FORCED = FORCED_STEP.report

  const show = (report: ResolutionReport = FORCED) =>
    render(<PdPreviewDiff report={report} decisions={{}} onChoose={vi.fn()} />)

  it('names every dependent a forced version breaks, before anything runs', () => {
    // DISCLAIMER §2. The heading counts dependents, not forced packages: two break here.
    show()
    const forced = section('forced')
    expect(within(forced).getByText('Will break (2)')).toBeInTheDocument()
    expect(within(forced).getByText('numpy → 2.5.1')).toBeInTheDocument()
    expect(within(forced).getByText('pandas 2.1.4 requires numpy<2,>=1.26.0')).toBeInTheDocument()
    expect(
      within(forced).getByText('scipy 1.11.4 requires numpy<1.28.0,>=1.21.6'),
    ).toBeInTheDocument()
  })

  it('says why a package was left out, with its range against this Python', () => {
    // Hard invariant 10 — not pip's "No matching distribution", which reads like a typo.
    show()
    expect(
      within(section('incompatible')).getByText(
        'numba 0.58.1 requires Python >=3.8,<3.12; this environment has 3.12.4.',
      ),
    ).toBeInTheDocument()
  })

  it('warns about a withdrawn release without treating it as a failure', () => {
    show()
    expect(
      within(section('yanked')).getByText(
        'oldlib 2.0.0 was withdrawn by its maintainers: declares the wrong Requires-Python',
      ),
    ).toBeInTheDocument()
    // Still planned: both engines install a yanked release (SP-2), so the change stays listed.
    expect(within(section('new-install')).getByText('oldlib')).toBeInTheDocument()
  })

  it('shows an unattributed blocker as its bare constraint, and a dependent without a version', () => {
    // ARCHITECTURE §3: when attribution is ambiguous the constraint is shown without a culprit
    // rather than guessed (`Blocker.by` absent), and the probe does not always know a dependent's
    // version. Each still counts once toward what forcing breaks.
    const numpy = FORCED.forced?.[0]
    if (numpy === undefined) throw new Error('the forced fixture forces numpy')
    show({
      ...FORCED,
      forced: [
        {
          ...numpy,
          blockers: [{ constraint: 'numpy<2' }, { by: 'pandas', constraint: 'numpy<2,>=1.26.0' }],
        },
      ],
      yanked: [{ pkg: 'oldlib', version: '2.0.0' }],
    })
    const forced = section('forced')
    expect(within(forced).getByText('Will break (2)')).toBeInTheDocument()
    expect(within(forced).getByText('numpy<2')).toBeInTheDocument()
    expect(within(forced).getByText('pandas requires numpy<2,>=1.26.0')).toBeInTheDocument()
    // A withdrawal with no reason given is still said, without a dangling colon.
    expect(
      within(section('yanked')).getByText('oldlib 2.0.0 was withdrawn by its maintainers.'),
    ).toBeInTheDocument()
  })

  it('heads a force that breaks nothing by what was forced, not by a count of zero', () => {
    const numpy = FORCED.forced?.[0]
    if (numpy === undefined) throw new Error('the forced fixture forces numpy')
    show({ ...FORCED, forced: [{ pkg: numpy.pkg, latest: numpy.latest, resolved: numpy.resolved }] })
    const forced = section('forced')
    expect(within(forced).getByText('Forced (1)')).toBeInTheDocument()
    expect(within(forced).queryByText(/Will break/)).toBeNull()
  })

  it('does not call a plan whose only content is a left-out package "nothing to change"', () => {
    // The user asked for numba and is owed the reason it is not happening.
    show({ changes: [], heldBack: [], incompatible: FORCED.incompatible ?? [] })
    expect(section('incompatible')).toBeInTheDocument()
    expect(screen.queryByText('nothing to change')).toBeNull()
  })
})
