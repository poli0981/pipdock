/**
 * Every screen and the components with structure of their own, through axe — TESTING §2.
 *
 * The suite before this asserted roles one at a time, wherever a test happened to reach for
 * `getByRole`. That holds what someone thought to check. This holds the rest: a `role="row"` whose
 * child is not a cell, a tab without a panel, an `aria-selected` on something that cannot be
 * selected. Each of those shipped, and each passed every test there was.
 *
 * Rendered through `<App />` where a screen is reachable that way, because the landmarks, the
 * heading order and the one-`<h1>` rule are properties of the shell and its screen together — the
 * seam a component test cannot see. `ui/src/test/axe.ts` says what axe itself cannot see.
 */

import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { App } from '@/App'
import { NAV_KEYS } from '@/components/nav'
import { PdAuditReport } from '@/components/PdAuditReport'
import { PdConflictRow } from '@/components/PdConflictRow'
import { PdHealthReport } from '@/components/PdHealthReport'
import { PdLegalGate } from '@/components/PdLegalGate'
import { PdPackageTable } from '@/components/PdPackageTable'
import { PdSummarySheet } from '@/components/PdSummarySheet'
import { PdUninstallDialog } from '@/components/PdUninstallDialog'
import type {
  AuditReport,
  Dist,
  EnvRow,
  ExecutionOutcome,
  ExecutionSummary,
  FlowStep,
  GuardReport,
  HealthReport,
  Pin,
  SnapshotMeta,
} from '@/ipc'
import { PdPlanPanel } from '@/screens/PdPlanPanel'
import { joinRows } from '@/screens/rows'
import { useEnvStore, useLegalStore, usePlanStore, useUiStore } from '@/stores'
import { groupRuff, HEALTH_TABS } from '@/stores/health'
import { violations } from '@/test/axe'
import auditFixture from '@/test/fixtures/audit_report.json'
import summaryFixture from '@/test/fixtures/execution_summary.json'
import flowStep from '@/test/fixtures/flow_step.json'
import flowStepForced from '@/test/fixtures/flow_step_forced.json'
import guardFixture from '@/test/fixtures/guard_report.json'
import healthFixture from '@/test/fixtures/health_report.json'
import pinFixture from '@/test/fixtures/pin_list.json'
import listFixture from '@/test/fixtures/pkg_list.json'
import outdatedFixture from '@/test/fixtures/pkg_outdated.json'
import snapshotFixture from '@/test/fixtures/snapshot_list.json'
import { resetStore } from '@/test/stores'

const { legalConsentGet } = vi.hoisted(() => ({ legalConsentGet: vi.fn() }))

vi.mock('@/ipc', async () => {
  const actual = await vi.importActual<typeof import('@/ipc')>('@/ipc')
  const { ipcMock } = await import('@/test/ipc')
  // The real module for its types and helpers, the spies over every command a screen calls on
  // mount, and the consent check the shell fails closed on (see App.test.tsx).
  return {
    ...actual,
    ...ipcMock(),
    legalConsentGet,
    // Environments subscribes on mount; the real one reaches for a Tauri host jsdom does not have.
    onScanProgress: vi.fn().mockResolvedValue(() => undefined),
  }
})
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn().mockResolvedValue(undefined) }))
vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({
  writeText: vi.fn().mockResolvedValue(undefined),
}))

const DISTS = listFixture as Dist[]
const INTERPRETER = String.raw`C:\proj\.venv\Scripts\python.exe`
const ROW: EnvRow = {
  interpreter: INTERPRETER,
  source: 'venv-scan',
  envHash: 'abc123',
  env: {
    interpreter: INTERPRETER,
    prefix: String.raw`C:\proj\.venv`,
    pythonVersion: '3.12.10',
    externallyManaged: false,
    hiddenUserSite: null,
    source: 'venv-scan',
  },
}
const JOINED = joinRows(DISTS, outdatedFixture, pinFixture as Pin[])
const HEALTH = healthFixture as HealthReport
const SUMMARY = summaryFixture as ExecutionSummary
const OUTCOME: ExecutionOutcome = {
  summary: SUMMARY,
  snapshot: (snapshotFixture as SnapshotMeta[])[0]!,
}

beforeEach(() => {
  vi.clearAllMocks()
  resetStore(usePlanStore)
  legalConsentGet.mockResolvedValue({ current: true, docsHash: 'a1b2c3d4e5f60718' })
  useLegalStore.setState({ accepted: true, review: false })
  useEnvStore.setState({
    rows: [ROW],
    selected: INTERPRETER,
    dists: DISTS,
    packages: JOINED.rows,
    listing: 'ready',
    outdatedStatus: 'ready',
    selection: new Set<string>(),
  })
})

describe('every screen, inside the shell', () => {
  it.each(NAV_KEYS)('%s', async (key) => {
    useUiStore.setState({ nav: key })
    render(<App />)
    await screen.findByRole('heading', { level: 1 })

    expect(await violations(document.body, { page: true })).toEqual([])
  })

  it('the command palette, open', async () => {
    useUiStore.setState({ nav: 'installed' })
    render(<App />)
    fireEvent.keyDown(window, { key: 'k', code: 'KeyK', ctrlKey: true })
    await screen.findByRole('combobox')

    expect(await violations(document.body, { page: true })).toEqual([])
  })

  it('the legal gate, which replaces the shell', async () => {
    render(<PdLegalGate />)
    await waitFor(() => {
      expect(screen.getByRole('dialog')).toBeInTheDocument()
    })

    expect(await violations()).toEqual([])
  })
})

describe('components with structure of their own', () => {
  it('the package grid, with pinned and outdated rows', async () => {
    render(
      <PdPackageTable
        rows={JOINED.rows}
        outdatedStatus="ready"
        selection={new Set(['pandas'])}
        onToggle={vi.fn()}
        onPinToggle={vi.fn()}
        onUninstall={vi.fn()}
        onDetails={vi.fn()}
        onSelectAll={vi.fn()}
        initialRect={{ width: 1280, height: 600 }}
      />,
    )

    expect(await violations()).toEqual([])
  })

  it.each(HEALTH_TABS)('the health report on its %s tab', async (tab) => {
    render(
      <PdHealthReport
        report={HEALTH}
        ruffByFile={groupRuff(HEALTH.ruff.findings)}
        tab={tab}
        onTab={vi.fn()}
        installed={['numpy', 'pandas']}
        onUninstall={vi.fn()}
      />,
    )

    expect(await violations()).toEqual([])
  })

  it('the audit report', async () => {
    render(<PdAuditReport report={auditFixture as AuditReport} />)

    expect(await violations()).toEqual([])
  })

  it('a conflict row and its three choices', async () => {
    const step = flowStep as FlowStep
    if (!('report' in step)) throw new Error('the fixture must be a step carrying a report')
    render(
      <ul>
        <PdConflictRow
          pkg="numpy"
          resolved="1.26.4"
          latest="2.5.1"
          blockers={step.report.heldBack?.[0]?.blockers ?? []}
          impossible={false}
          value={undefined}
          onChoose={vi.fn()}
        />
      </ul>,
    )

    expect(await violations()).toEqual([])
  })

  it('the uninstall guard', async () => {
    render(
      <PdUninstallDialog
        report={guardFixture as GuardReport}
        busy={false}
        onCancel={vi.fn()}
        onWiden={vi.fn()}
        onConfirm={vi.fn()}
      />,
    )

    expect(await violations()).toEqual([])
  })

  it('the plan panel, previewing a forced plan', async () => {
    usePlanStore.setState({
      phase: 'preview',
      kind: 'update',
      step: flowStepForced as FlowStep,
      env: ROW.env ?? null,
    })
    render(<PdPlanPanel onFinished={vi.fn()} />)

    expect(await violations()).toEqual([])
  })

  it('the plan panel, on its summary', async () => {
    usePlanStore.setState({ phase: 'summary', outcome: OUTCOME, kind: 'update' })
    render(<PdPlanPanel onFinished={vi.fn()} />)

    expect(await violations()).toEqual([])
  })

  it('the summary sheet alone', async () => {
    render(<PdSummarySheet outcome={OUTCOME} onDone={vi.fn()} />)

    expect(await violations()).toEqual([])
  })
})
