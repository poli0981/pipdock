/**
 * `PdPlanPanel`'s summary phase — the one screen no other test renders together with its child.
 *
 * `PdSummarySheet` has its own tests and the panel had none, so the two were never on screen at
 * the same time in a test. That is exactly where the bug lived: both rendered `plan.done`, and a
 * successful run showed *Back to packages* twice, one under the other. Reported from the
 * installed build.
 */

import { render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { PdPlanPanel } from '@/screens/PdPlanPanel'
import type { ExecutionOutcome, ExecutionSummary, SnapshotMeta } from '@/ipc'
import { usePlanStore } from '@/stores/plan'
import { resetStore } from '@/test/stores'
import snapshotFixture from '@/test/fixtures/snapshot_list.json'
import summaryFixture from '@/test/fixtures/execution_summary.json'

vi.mock('@/ipc', async () => {
  const { ipcMock } = await import('@/test/ipc')
  return ipcMock()
})

const SUMMARY = summaryFixture as ExecutionSummary

const OUTCOME: ExecutionOutcome = {
  summary: SUMMARY,
  // From the committed fixture rather than hand-written. A literal here needs every field of
  // `SnapshotMeta` and invents none — mine was missing `appVersion` and had three fields that do
  // not exist, which `npm test` happily accepted and `tsc` caught in CI.
  snapshot: (snapshotFixture as SnapshotMeta[])[0]!,
}


describe('the summary phase', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    resetStore(usePlanStore)
  })

  it('offers exactly one way back after a successful run', () => {
    usePlanStore.setState({ phase: 'summary', outcome: OUTCOME, kind: 'update' })
    render(<PdPlanPanel onFinished={vi.fn()} />)

    // One, not two. The sheet renders it; the panel must not render a second beneath.
    expect(screen.getAllByText('Back to packages')).toHaveLength(1)
  })

  it('still offers a way back when the run failed before producing a summary', () => {
    // No `ExecutionSummary` means no sheet, so without the panel's own button this phase would
    // be a dead end — the error is on screen and nothing dismisses it.
    usePlanStore.setState({
      phase: 'summary',
      outcome: null,
      error: { code: 'PD-SNP-001', message: 'snapshot failed' },
    })
    render(<PdPlanPanel onFinished={vi.fn()} />)

    expect(screen.getAllByText('Back to packages')).toHaveLength(1)
  })
})
