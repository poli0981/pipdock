/**
 * The Security tab's list — PRD P1-1.
 *
 * Fed from `audit_report.json`, which is **computed** by `fixtures::audit_report` running the real
 * `audit::parse` over a real pip-audit capture of the SP-4 environment. Slice 0 is why: a
 * hand-written fixture that matches the documentation while the code does not makes a test assert
 * the right thing and prove the wrong one. Dedup and the fixable-first order are exactly what these
 * assertions read, and both live in Rust.
 *
 * The capture has two packages on purpose — one cannot show grouping.
 */

import { render, screen, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { ADVISORY_ROWS_SHOWN, PdAuditReport } from '@/components/PdAuditReport'
import type { AuditReport } from '@/ipc'
import auditFixture from '@/test/fixtures/audit_report.json'

const REPORT = auditFixture as AuditReport
const ADVISORIES = REPORT.advisories ?? []

const group = (pkg: string) => {
  const el = document.querySelector(`[data-pkg="${pkg}"]`)
  if (el === null) throw new Error(`no group rendered for ${pkg}`)
  return el as HTMLElement
}

describe('PdAuditReport', () => {
  it('groups advisories under the package they are against', () => {
    render(<PdAuditReport report={REPORT} />)

    // Thirteen deduplicated from sixteen rows, across the two packages the environment has.
    expect(ADVISORIES).toHaveLength(13)
    expect(within(group('urllib3')).getAllByRole('listitem')).toHaveLength(8)
    expect(within(group('pip')).getAllByRole('listitem')).toHaveLength(5)
  })

  it('reports pip itself, because the freeze is taken with --all', () => {
    // Not incidental. `Engine::freeze` passes `--all` on pip and uv has no such flag, so the
    // configured engine changes what is audited — and pip being in its own results is the most
    // visible instance of that.
    render(<PdAuditReport report={REPORT} />)

    expect(within(group('pip')).getByText(/^pip 25\.0\.1/)).toBeInTheDocument()
  })

  it('shows the CVE from the aliases, never as the id', () => {
    // PRD P1-1 says "known CVEs"; pip-audit's primary id is a PYSEC under the default service, so
    // a screen that read `id` alone would never show a CVE at all.
    render(<PdAuditReport report={REPORT} />)

    const row = screen.getByText('PYSEC-2023-192').closest('li')
    expect(row).not.toBeNull()
    expect(within(row as HTMLElement).getByText(/CVE-2023-43804/)).toBeInTheDocument()
  })

  it('says "no fix" rather than leaving the badge blank', () => {
    // An advisory nothing upgrades away is the one worth reading the description for, so the
    // absence is stated. Built here because the captured environment happens to have a fix for
    // everything — asserting on a state the fixture cannot reach would be asserting on nothing.
    const first = ADVISORIES[0]
    if (first === undefined) throw new Error('the fixture must carry an advisory')
    const unfixable: AuditReport = {
      ...REPORT,
      advisories: [{ ...first, fixVersions: [] }],
    }
    render(<PdAuditReport report={unfixable} />)

    expect(screen.getByText('no fix')).toBeInTheDocument()
  })

  it('links every advisory to its OSV entry', () => {
    render(<PdAuditReport report={REPORT} />)

    expect(screen.getAllByRole('button', { name: 'OSV entry' })).toHaveLength(13)
    expect(ADVISORIES.every((a) => a.url?.startsWith('https://osv.dev/vulnerability/'))).toBe(true)
  })

  it('caps the list without misreporting the total', () => {
    // The `RUFF_ROWS_SHOWN` guarantee: the count comes from the report, never from the rows on
    // screen, so a capped view cannot tell the user there are fewer problems than there are.
    render(<PdAuditReport report={REPORT} rowsShown={3} />)

    expect(screen.getAllByRole('listitem')).toHaveLength(3)
    expect(screen.getByText(/13 advisories across 2 of 2 packages/)).toBeInTheDocument()
    expect(screen.getByText(/10 more advisories are not shown/)).toBeInTheDocument()
    expect(ADVISORY_ROWS_SHOWN).toBe(200)
  })

  it('says nothing was found only when a report says so', () => {
    // Reachable only with a report in hand. "Nothing was found" and "nothing has run" are
    // different claims, and the screen — not this component — owns the second.
    render(<PdAuditReport report={{ ...REPORT, advisories: [] }} />)

    expect(screen.getByText(/No known advisories against 2 packages/)).toBeInTheDocument()
    expect(screen.queryByRole('listitem')).toBeNull()
  })
})
