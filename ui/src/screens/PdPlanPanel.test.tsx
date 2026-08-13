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
import type { ExecutionSummary } from '@/ipc'
import { usePlanStore } from '@/stores/plan'
import { resetStore } from '@/test/stores'
import summaryFixture from '@/test/fixtures/execution_summary.json'

vi.mock('@/ipc', async () => {
  const { ipcMock } = await import('@/test/ipc')
  return ipcMock()
})

const SUMMARY = summaryFixture as ExecutionSummary

const OUTCOME = {
  summary: SUMMARY,
  snapshot: {
    id: '2026-08-13T10-00-00Z',
    createdAt: '2026-08-13T10:00:00Z',
    engine: 'pip' as const,
    trigger: { plan: { planId: SUMMARY.planId } },
    packageCount: 6,
    envHash: 'aaa',
    interpreter: 'C:\\venv\\Scripts\\python.exe',
    pythonVersion: '3.12.4',
  },
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
