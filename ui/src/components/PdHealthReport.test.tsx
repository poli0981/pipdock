/**
 * `PdHealthReport` — the tabs, the three states a tab can be in, and the two gates on the
 * deptry handoff.
 *
 * Fed from `health_report.json` and `health_partial.json`, serialized from the real
 * `HealthReport` by `cargo run -p xtask -- ipc-fixtures` and held current by a Rust-side
 * staleness test. Every branch asserted below has a committed subject, guarded by
 * `the_health_fixtures_still_cover_every_branch_the_tabs_implement`.
 */

import { fireEvent, render, screen, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { PdHealthReport } from '@/components/PdHealthReport'
import type { HealthReport } from '@/ipc'
import { groupRuff, type HealthTab } from '@/stores/health'
import partial from '@/test/fixtures/health_partial.json'
import full from '@/test/fixtures/health_report.json'

vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn().mockResolvedValue(undefined) }))
const { openUrl } = await import('@tauri-apps/plugin-opener')

const REPORT = full as HealthReport
const PARTIAL = partial as HealthReport

// The names `pkg_list.json` carries, which is what the real screen passes.
const INSTALLED = ['certifi', 'editable-lib', 'numpy', 'pandas', 'requests', 'scipy']

function setup(
  report: HealthReport = REPORT,
  tab: HealthTab = 'deptry',
  installed: readonly string[] = INSTALLED,
) {
  const onTab = vi.fn()
  const onUninstall = vi.fn()
  render(
    <PdHealthReport
      report={report}
      ruffByFile={groupRuff(report.ruff.findings)}
      tab={tab}
      onTab={onTab}
      installed={installed}
      onUninstall={onUninstall}
    />,
  )
  return { onTab, onUninstall }
}

describe('tabs', () => {
  it('labels each tab with the tool that produced it and its count', () => {
    setup()
    // The visible label is the tool's own name — data, never translated (I18N §2).
    const tabs = screen.getAllByRole('tab')
    expect(tabs.map((t) => t.textContent)).toEqual([
      `deptry${REPORT.deptry.length}`,
      `vulture${REPORT.vulture.length}`,
      `ruff${REPORT.ruff.findings.length}`,
    ])
  })

  it('takes its counts from the report rather than recomputing them', () => {
    // A tab that counted its own rows could disagree with `pipdock health --json` over the same
    // run, which is the one difference nobody would think to check.
    setup()
    const ruffTab = screen.getAllByRole('tab')[2]
    expect(ruffTab?.textContent).toContain(String(REPORT.ruff.findings.length))
  })
})

describe('the three states a tab can be in', () => {
  it('shows findings for a tool that reported some', () => {
    setup(REPORT, 'deptry')
    // Two of them: the fixture carries an unused dependency that is installed and one that is
    // not, because the Uninstall button needs a subject on both sides of its gate.
    expect(screen.getAllByText('DEP002')).toHaveLength(2)
  })

  it('says "not run" for a tool that was never asked', () => {
    setup(PARTIAL, 'vulture')
    // `PdEmptyState` prefixes UI-SPEC §7's mono glyph, so the message is not the whole node.
    expect(screen.getByText(/not run/)).toBeInTheDocument()
  })

  it('says "did not finish" for a tool that failed — not "no issues found"', () => {
    // The bug this component would otherwise ship. `ran` is filled before the tool loop, so a
    // quarantined ruff.exe is in `ran` with an empty findings array, and a tab keyed on `ran`
    // alone renders it as clean. That is P3's own exit-criterion scenario shown as a lie.
    expect(PARTIAL.ran).toContain('ruff')
    setup(PARTIAL, 'ruff')
    expect(screen.getByText(/did not finish/)).toBeInTheDocument()
    expect(screen.queryByText(/no issues found/)).toBeNull()
  })
})

describe('the deptry handoff', () => {
  it('offers Uninstall only for an unused dependency that is installed', () => {
    const { onUninstall } = setup(REPORT, 'deptry')
    const rows = screen.getAllByRole('listitem')

    const requests = rows.find((r) => within(r).queryByText('requests'))
    expect(requests).toBeDefined()
    fireEvent.click(within(requests as HTMLElement).getByRole('button'))
    expect(onUninstall).toHaveBeenCalledWith('requests')
  })

  it('offers nothing for a name that is not an installed distribution', () => {
    // deptry names a *module*: `yaml` is `PyYAML`, `cv2` is `opencv-python`. Handing the guard a
    // module name produces PD-PKG-002 on a package that is installed, so an unknown name gets no
    // button rather than a broken one.
    setup(REPORT, 'deptry')
    const row = screen
      .getAllByRole('listitem')
      .find((r) => within(r).queryByText('httpx'))
    expect(row).toBeDefined()
    expect(within(row as HTMLElement).queryByRole('button')).toBeNull()
  })

  it('offers nothing for a missing dependency, even when the name matches', () => {
    // The gate every draft of this dropped. DEP001 is *missing* — a button offering to uninstall
    // it would tell the user to remove something the project does not have.
    setup(REPORT, 'deptry', [...INSTALLED, 'yaml'])
    const row = screen.getAllByRole('listitem').find((r) => within(r).queryByText('yaml'))
    expect(row).toBeDefined()
    expect(within(row as HTMLElement).queryByRole('button')).toBeNull()
  })

  it('discloses that deptry compared against the wrong environment', () => {
    // CODE-HEALTH-SPEC §3 as amended requires this "where findings are shown", and the CLI
    // prints it under the same condition. Two heads, one caveat.
    setup(REPORT, 'deptry')
    expect(screen.getByText(/DEP001 and DEP003 can be swapped/)).toBeInTheDocument()
  })
})

describe('the ruff tab', () => {
  it('groups findings by file', () => {
    setup(REPORT, 'ruff')
    const files = new Set(REPORT.ruff.findings.map((f) => f.filename))
    for (const file of files) {
      expect(screen.getByText(file)).toBeInTheDocument()
    }
  })

  it('opens the rule page ruff itself carried', () => {
    // Constructed URLs 404: the page is keyed by rule *name*, not by code. This asserts the
    // component hands over `finding.url` rather than building one from `finding.code`.
    setup(REPORT, 'ruff')
    const withUrl = REPORT.ruff.findings.find((f) => f.url != null)
    expect(withUrl).toBeDefined()
    fireEvent.click(screen.getByText(withUrl?.code ?? ''))
    expect(openUrl).toHaveBeenCalledWith(withUrl?.url)
  })

  it('renders a finding with no rule page as plain text, not a dead link', () => {
    setup(REPORT, 'ruff')
    const syntax = REPORT.ruff.findings.find((f) => f.url == null)
    expect(syntax).toBeDefined()
    const label = screen.getByText(syntax?.code ?? '')
    expect(label.tagName).toBe('CODE')
  })

  it('badges only the fixes ruff would actually apply', () => {
    setup(REPORT, 'ruff')
    // `fixable` counts safe fixes only, and the badge count must match the number Rust reported
    // — the same one P5's dialog and the CLI prompt will name.
    expect(screen.getAllByText('fixable')).toHaveLength(REPORT.ruff.fixable)
    expect(screen.getAllByText('unsafe fix').length).toBeGreaterThan(0)
  })

  it('caps a very long list and says how many it withheld', () => {
    render(
      <PdHealthReport
        report={REPORT}
        ruffByFile={groupRuff(REPORT.ruff.findings)}
        tab="ruff"
        onTab={vi.fn()}
        installed={INSTALLED}
        rowsShown={2}
      />,
    )
    const withheld = REPORT.ruff.findings.length - 2
    expect(screen.getByText(`Show ${withheld} more findings`)).toBeInTheDocument()
  })
})

describe('the footer', () => {
  it('says notebooks are excluded, which nothing on the wire carries', () => {
    setup()
    expect(screen.getByText(/\.ipynb/)).toBeInTheDocument()
  })

  it('names where the project declares its dependencies', () => {
    setup()
    expect(screen.getByText(/requirements-dev\.txt, requirements\.txt/)).toBeInTheDocument()
  })
})
