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
import type { ExecutionOutcome, ExecutionSummary, FlowStep, PyEnv, SnapshotMeta } from '@/ipc'
import { useSettingsStore } from '@/stores'
import { usePlanStore } from '@/stores/plan'
import { resetStore } from '@/test/stores'
import flowStepForced from '@/test/fixtures/flow_step_forced.json'
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

const FORCED_STEP = flowStepForced as FlowStep

const ENV: PyEnv = {
  interpreter: 'C:/work/.venv/Scripts/python.exe',
  prefix: 'C:/work/.venv',
  pythonVersion: '3.12.4',
  externallyManaged: false,
  hiddenUserSite: null,
  source: 'venv-scan',
}

/** The panel showing the forced plan's preview, for `env`. */
function preview(env: PyEnv = ENV) {
  usePlanStore.setState({ phase: 'preview', kind: 'update', step: FORCED_STEP, env })
  render(<PdPlanPanel onFinished={vi.fn()} />)
}

describe('a forced plan', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    resetStore(usePlanStore)
    resetStore(useSettingsStore)
  })

  it('counts what it breaks on the button that does it', () => {
    // DISCLAIMER §2: the cost is on the control, not only in a section above the fold.
    preview()
    const confirm = screen.getByRole('button', { name: 'Confirm (2 changes, breaks 2)' })
    expect(confirm).toHaveClass('text-danger')
  })

  it('names the environment it changes', () => {
    // U15: the panel replaces the screen it was started from, so nothing else on screen says.
    preview()
    expect(screen.getByText(ENV.interpreter)).toBeInTheDocument()
  })
})

describe('the PEP 668 chip', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    resetStore(usePlanStore)
    resetStore(useSettingsStore)
  })

  it('is on screen while the override applies to the environment being changed', () => {
    useSettingsStore.setState({ allowExternallyManaged: true })
    preview({ ...ENV, externallyManaged: true })
    expect(screen.getByText('PEP 668 override')).toBeInTheDocument()
  })

  it('is absent where the override changes nothing', () => {
    // A venv is never externally managed, and a chip there would claim the flag is being sent.
    useSettingsStore.setState({ allowExternallyManaged: true })
    preview(ENV)
    expect(screen.queryByText('PEP 668 override')).toBeNull()
  })

  it('is absent while the override is off, even for a marked environment', () => {
    preview({ ...ENV, externallyManaged: true })
    expect(screen.queryByText('PEP 668 override')).toBeNull()
  })
})
