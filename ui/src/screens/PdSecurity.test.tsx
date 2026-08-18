/**
 * The Security screen — the states around the list, which `PdAuditReport.test.tsx` does not cover.
 *
 * It exists because of what it caught. Driving the screen against a deliberately slow bridge showed
 * the body reading **"No audit has run for this environment"** *while an audit was running*: the
 * report is cleared before the command, so `shown` is null during a run, and the un-run empty state
 * was rendered on that basis. That is P4's "no issues found before anything had run" inverted — a
 * true-looking sentence derived from a state the screen had not loaded — and no test saw it,
 * because every other test renders this screen settled.
 */

import { render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { PdSecurity } from '@/screens/PdSecurity'
import type { AuditReport } from '@/ipc'
import { useEnvStore, useSecurityStore } from '@/stores'
import auditFixture from '@/test/fixtures/audit_report.json'

vi.mock('@/ipc', async () => {
  const actual = await vi.importActual<typeof import('@/ipc')>('@/ipc')
  return { ...actual, auditRun: vi.fn(), auditCancel: vi.fn(), onAuditProgress: vi.fn() }
})

const REPORT = auditFixture as AuditReport
const ENV_HASH = 'abc123'

const ROW = {
  interpreter: String.raw`C:\proj\.venv\Scripts\python.exe`,
  source: 'venv' as const,
  envHash: ENV_HASH,
  env: {
    interpreter: String.raw`C:\proj\.venv\Scripts\python.exe`,
    prefix: String.raw`C:\proj\.venv`,
    pythonVersion: '3.12.10',
    externallyManaged: false,
    source: 'venv' as const,
  },
}

beforeEach(() => {
  useSecurityStore.setState({
    phase: 'idle',
    report: null,
    reportFor: null,
    error: null,
    console: [],
    done: 0,
    total: 0,
    current: null,
  })
  useEnvStore.setState({ rows: [ROW], selected: ROW.interpreter } as never)
})

describe('PdSecurity', () => {
  it('says nothing has run when nothing has', () => {
    render(<PdSecurity />)

    expect(screen.getByText(/No audit has run for this environment/)).toBeInTheDocument()
  })

  it('does not claim nothing has run while a run is going', () => {
    // The defect this file was written for. Asserted as an absence, so it fails if the un-run
    // empty state comes back — which it would, because `shown` is legitimately null here.
    useSecurityStore.setState({ phase: 'running' })
    render(<PdSecurity />)

    expect(screen.queryByText(/No audit has run/)).toBeNull()
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Run audit' })).toBeDisabled()
  })

  it('shows the report only for the environment it belongs to', () => {
    // `reportFor` is the whole reason the store keeps it: a plain `report` read would leave one
    // environment's advisories on screen after the user switched to another.
    useSecurityStore.setState({ phase: 'ready', report: REPORT, reportFor: 'a-different-env' })
    render(<PdSecurity />)

    expect(screen.queryByText(/13 advisories/)).toBeNull()
    expect(screen.getByText(/No audit has run for this environment/)).toBeInTheDocument()
  })

  it('renders the advisories once the report matches', () => {
    useSecurityStore.setState({ phase: 'ready', report: REPORT, reportFor: ENV_HASH })
    render(<PdSecurity />)

    expect(screen.getByText(/13 advisories across 2 of 2 packages/)).toBeInTheDocument()
  })

  it('says a cancelled run was cancelled, and keeps what it got', () => {
    // A cancel is a state, not an error: no error row, and the partial result stays on screen.
    useSecurityStore.setState({
      phase: 'ready',
      report: { ...REPORT, cancelled: true },
      reportFor: ENV_HASH,
    })
    render(<PdSecurity />)

    expect(screen.getByText(/Stopped before it finished/)).toBeInTheDocument()
    expect(screen.getByText(/13 advisories/)).toBeInTheDocument()
  })
})
