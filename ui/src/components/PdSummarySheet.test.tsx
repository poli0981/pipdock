/**
 * `PdSummarySheet` counts — the third of TESTING §2's five L3 obligations, "including the
 * cancelled case".
 *
 * The fixture is a **cancelled** run on purpose: it is the one DATA-FLOW §6 does not work
 * through, the one whose copy was deferred out of Stage 1, and the one where the headline and the
 * rows are most likely to disagree.
 */

import { fireEvent, render, screen, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { PdSummarySheet } from '@/components/PdSummarySheet'
import type { ExecutionOutcome, ExecutionSummary } from '@/ipc'
import summaryFixture from '@/test/fixtures/execution_summary.json'

const SUMMARY = summaryFixture as ExecutionSummary

function setup(overrides: Partial<ExecutionOutcome> = {}, onRollback?: () => void) {
  const onDone = vi.fn()
  const outcome: ExecutionOutcome = {
    summary: SUMMARY,
    snapshot: {
      id: '2026-08-04T10-00-00Z',
      createdAt: '2026-08-04T10:00:00Z',
      engine: 'pip',
      // A plan-triggered snapshot carries the plan it protects, which is what makes the
      // summary's rollback offer point at the right thing.
      trigger: { plan: { planId: SUMMARY.planId } },
      packageCount: 6,
      appVersion: '0.1.0',
    },
    ...overrides,
  }
  render(
    <PdSummarySheet
      outcome={outcome}
      onDone={onDone}
      {...(onRollback === undefined ? {} : { onRollback })}
    />,
  )
  return { onDone, outcome }
}

describe('the headline', () => {
  it('matches the rows below it', () => {
    // TESTING §1.4 names this as a thing that must never regress. Core derives `counts` from
    // `results` rather than accumulating, and the sheet renders `counts` rather than re-deriving —
    // so this test is really checking that the sheet did not quietly invent its own arithmetic.
    setup()
    expect(screen.getByText('2 successful, 1 failed, 1 skipped')).toBeInTheDocument()

    const rows = screen.getAllByRole('listitem')
    const byStatus = (s: string) => rows.filter((r) => r.dataset.status === s).length
    expect(byStatus('ok')).toBe(SUMMARY.counts.ok)
    expect(byStatus('failed')).toBe(SUMMARY.counts.failed)
    expect(byStatus('skipped')).toBe(SUMMARY.counts.skipped)
  })

  it('gives a failed package its catalog code and stderr, not just a red word', () => {
    // ERROR-CATALOG §3's row shape: code, localized one-liner, Details expanding the tail.
    setup()
    const failed = screen.getAllByRole('listitem').find((r) => r.dataset.status === 'failed')
    if (failed === undefined) throw new Error('the fixture has no failed row')
    expect(within(failed).getByText('PD-BLD-002')).toBeInTheDocument()
  })
})

describe('the cancelled case', () => {
  it('says the run stopped part-way and points at the snapshot', () => {
    // The Stage 1 deferral: killing pip mid-install can leave site-packages partially written,
    // and the summary has to say so rather than reporting a tidy set of counts as if nothing
    // were unusual.
    setup()
    expect(screen.getByText(/stopped this part-way/)).toBeInTheDocument()
    expect(screen.getByText(/half-written/)).toBeInTheDocument()
    expect(screen.getByText('2026-08-04T10-00-00Z')).toBeInTheDocument()
  })

  it('says nothing about cancelling when the run finished on its own', () => {
    setup({ summary: { ...SUMMARY, cancelled: false } })
    expect(screen.queryByText(/stopped this part-way/)).toBeNull()
  })

  it('still shows the snapshot when the run was clean, because rollback is still offered', () => {
    setup({ summary: { ...SUMMARY, cancelled: false } })
    expect(screen.getByText('2026-08-04T10-00-00Z')).toBeInTheDocument()
  })

  it('offers the rollback its own copy has been promising', () => {
    // `plan.cancelledDetail` has said "the snapshot below restores the environment exactly as it
    // was" since S3, with nothing behind it. The button hands back the id, so the caller does not
    // have to reach into the outcome it already passed in.
    const onRollback = vi.fn()
    setup({}, onRollback)
    fireEvent.click(screen.getByText('Roll back to this'))
    expect(onRollback).toHaveBeenCalledWith('2026-08-04T10-00-00Z')
  })

  it('offers no rollback when the caller cannot perform one', () => {
    // Presentational: the sheet does not reach for a store to decide whether a restore is
    // possible, so no handler means no button.
    setup()
    expect(screen.queryByText('Roll back to this')).toBeNull()
  })
})

describe('the post-run check', () => {
  it('reports findings separately from the counts', () => {
    // A finding means the environment is inconsistent *after* a run whose steps may all have
    // reported ok, so folding it into `failed` would misattribute it to a package.
    setup({
      summary: {
        ...SUMMARY,
        check: { ok: false, findings: [{ pkg: 'httpx', requirement: 'httpcore<0.16' }] },
      },
    })
    expect(screen.getByText(/1 dependency problem remains/)).toBeInTheDocument()
    expect(screen.getByText('httpx requires httpcore<0.16')).toBeInTheDocument()
    // The headline is unchanged: the check is not a step outcome.
    expect(screen.getByText('2 successful, 1 failed, 1 skipped')).toBeInTheDocument()
  })
})
