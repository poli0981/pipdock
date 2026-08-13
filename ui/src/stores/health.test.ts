/**
 * `useHealthStore` — the grouping, the three tab states, and the pair the report is keyed to.
 *
 * The rules here are the ones a component test cannot see: which key a report is valid under,
 * what a tab shows when its tool failed rather than found nothing, and that a re-run subscribes
 * before it invokes. All three fail *silently* — the wrong report renders, an empty tab lies, or
 * a bootstrap streams into nothing — so none of them shows up as an error anywhere.
 */

import { beforeEach, describe, expect, it, vi } from 'vitest'

import * as ipc from '@/ipc'
import type { HealthReport, PyEnv, RuffFinding } from '@/ipc'
import { freshReport, groupRuff, tabState, useHealthStore } from '@/stores/health'
import { resetStore } from '@/test/stores'
import partial from '@/test/fixtures/health_partial.json'
import full from '@/test/fixtures/health_report.json'

vi.mock('@/ipc', async () => {
  const { ipcMock } = await import('@/test/ipc')
  return ipcMock()
})

const REPORT = full as HealthReport
const PARTIAL = partial as HealthReport

const ENV: PyEnv = {
  interpreter: 'C:\\venv\\Scripts\\python.exe',
  prefix: 'C:\\venv',
  pythonVersion: '3.12.4',
  externallyManaged: false,
  hiddenUserSite: null,
  source: 'manual',
}

const finding = (filename: string, fix: RuffFinding['fix']): RuffFinding => ({
  name: 'unused-import',
  message: '`os` imported but unused',
  filename,
  row: 1,
  column: 8,
  ...(fix === undefined ? {} : { fix }),
})

describe('groupRuff', () => {
  it('keeps the order ruff reported the files in', () => {
    const groups = groupRuff([
      finding('b.py', 'safe'),
      finding('a.py', 'safe'),
      finding('b.py', 'safe'),
    ])
    expect(groups.map((g) => g.file)).toEqual(['b.py', 'a.py'])
    expect(groups[0]?.findings).toHaveLength(2)
  })

  it('counts only the fixes ruff would actually apply', () => {
    // `unsafe` needs --unsafe-fixes, which PipDock never passes, and `display` is never applied
    // at all. A badge counting them promises a fix the fix will not deliver.
    const groups = groupRuff([
      finding('a.py', 'safe'),
      finding('a.py', 'unsafe'),
      finding('a.py', 'display'),
      finding('a.py', undefined),
    ])
    expect(groups[0]?.findings).toHaveLength(4)
    expect(groups[0]?.fixable).toBe(1)
  })

  it('agrees with the counts the report already carries', () => {
    // The UI never recomputes `fixable`/`fixableFiles` — Rust counts them so the GUI, the CLI and
    // P5's refusal name one number. This asserts the grouping cannot silently disagree with them.
    const groups = groupRuff(REPORT.ruff.findings)
    expect(groups.reduce((n, g) => n + g.fixable, 0)).toBe(REPORT.ruff.fixable)
    expect(groups.filter((g) => g.fixable > 0)).toHaveLength(REPORT.ruff.fixableFiles)
  })
})

describe('tabState', () => {
  it('calls a tool that reported nothing clean', () => {
    expect(tabState(REPORT, 'deptry', REPORT.deptry.length)).toBe('findings')
    expect(tabState({ ...REPORT, problems: [] }, 'vulture', 0)).toBe('clean')
  })

  it('never calls a tool that was not asked to run clean', () => {
    expect(tabState(PARTIAL, 'vulture', 0)).toBe('notRun')
  })

  it('never calls a tool that failed clean, even though it is in `ran`', () => {
    // The trap the whole three-state rule exists for. `health::run` fills `ran` from the
    // selection *before* the tool loop, so a failed tool is in `ran` with an empty findings
    // array — and `ran.includes(tool)` alone would render a quarantined ruff.exe as "no lint
    // findings", which is P3's own exit-criterion scenario shown as a lie.
    expect(PARTIAL.ran).toContain('ruff')
    expect(PARTIAL.ruff.findings).toHaveLength(0)
    expect(tabState(PARTIAL, 'ruff', 0)).toBe('failed')
  })
})

describe('freshReport', () => {
  const key = { report: REPORT, reportFor: { envHash: 'aaa', folder: 'C:\\proj' }, folder: 'C:\\proj' }

  it('returns the report when both halves of the key match', () => {
    expect(freshReport(key, 'aaa')).toBe(REPORT)
  })

  it('withholds it when the environment moved', () => {
    expect(freshReport(key, 'bbb')).toBeNull()
  })

  it('withholds it when the folder moved', () => {
    // Two environments can legitimately name the same folder, so neither half is the key alone.
    expect(freshReport({ ...key, folder: 'C:\\other' }, 'aaa')).toBeNull()
  })
})

describe('useHealthStore', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    resetStore(useHealthStore)
  })

  it('keeps the folder it resets around', () => {
    const store = useHealthStore.getState()
    store.setFolder('C:\\proj')
    useHealthStore.setState({ report: REPORT, reportFor: { envHash: 'aaa', folder: 'C:\\proj' } })

    useHealthStore.getState().setFolder('C:\\other')
    const after = useHealthStore.getState()
    expect(after.folder).toBe('C:\\other')
    expect(after.report).toBeNull()
    expect(after.reportFor).toBeNull()
  })

  it('does not reset when the folder is chosen again', () => {
    useHealthStore.getState().setFolder('C:\\proj')
    useHealthStore.setState({ report: REPORT })
    useHealthStore.getState().setFolder('C:\\proj')
    expect(useHealthStore.getState().report).toBe(REPORT)
  })

  it('subscribes to progress before it invokes the command', async () => {
    // On a cold install the tools-venv bootstrap is the slowest part of the run and is emitted
    // *first*. Subscribing after the invoke loses exactly the progress the user is waiting on.
    const order: string[] = []
    vi.mocked(ipc.onHealthProgress).mockImplementation(() => {
      order.push('subscribe')
      return Promise.resolve(() => undefined)
    })
    vi.mocked(ipc.healthRun).mockImplementation(() => {
      order.push('invoke')
      return Promise.resolve(REPORT)
    })

    useHealthStore.getState().setFolder('C:\\proj')
    await useHealthStore.getState().run(ENV, 'aaa')

    expect(order).toEqual(['subscribe', 'invoke'])
  })

  it('groups ruff once, when the report lands', async () => {
    vi.mocked(ipc.onHealthProgress).mockResolvedValue(() => undefined)
    vi.mocked(ipc.healthRun).mockResolvedValue(REPORT)

    useHealthStore.getState().setFolder('C:\\proj')
    await useHealthStore.getState().run(ENV, 'aaa')

    const state = useHealthStore.getState()
    expect(state.phase).toBe('ready')
    expect(state.reportFor).toEqual({ envHash: 'aaa', folder: 'C:\\proj' })

    // One group per distinct file, so the tab's sections match what ruff actually reported.
    const files = new Set(REPORT.ruff.findings.map((f) => f.filename))
    expect(state.ruffByFile.map((g) => g.file).sort()).toEqual([...files].sort())

    // Held as state: two reads with no `set` between them return the *same array*. A selector
    // calling `groupRuff` would return a fresh one each time, which is what hands React a new
    // reference on every `getSnapshot` and either warns or re-renders forever.
    expect(useHealthStore.getState().ruffByFile).toBe(useHealthStore.getState().ruffByFile)
  })

  it('leaves a failed run on screen with its code', async () => {
    // `idle` here would unmount whatever renders the error, and the user would press Run and see
    // nothing happen at all — the bug `PlanPhase.failed` was added for.
    vi.mocked(ipc.onHealthProgress).mockResolvedValue(() => undefined)
    vi.mocked(ipc.healthRun).mockRejectedValue({ code: 'PD-HLT-004', message: 'venv failed' })

    useHealthStore.getState().setFolder('C:\\proj')
    await useHealthStore.getState().run(ENV, 'aaa')

    const state = useHealthStore.getState()
    expect(state.phase).toBe('failed')
    expect(state.error?.code).toBe('PD-HLT-004')
  })

  it('refuses a second run while one is in flight', async () => {
    vi.mocked(ipc.onHealthProgress).mockResolvedValue(() => undefined)
    vi.mocked(ipc.healthRun).mockResolvedValue(REPORT)

    useHealthStore.getState().setFolder('C:\\proj')
    useHealthStore.setState({ phase: 'running' })
    await useHealthStore.getState().run(ENV, 'aaa')

    // Rust answers a second claim with PD-RES-003; not sending it at all is the better error.
    expect(ipc.healthRun).not.toHaveBeenCalled()
  })

  it('does nothing without a folder', async () => {
    await useHealthStore.getState().run(ENV, 'aaa')
    expect(ipc.healthRun).not.toHaveBeenCalled()
  })
})
