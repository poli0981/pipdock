/**
 * The Security tab's findings — PRD P1-1, SECURITY §6.
 *
 * Prop-driven and store-free, the division `PdHealthReport` uses: the screen owns the run, the
 * errors and the progress, this owns the list. That is what lets it be tested against a generated
 * fixture with no store at all.
 *
 * **Everything pip-audit said is rendered verbatim.** There is no PipDock copy per advisory id and
 * no rewritten description — I18N §2 keeps ids, versions and tool output out of the catalogs.
 *
 * Two things it deliberately does not do:
 *
 * 1. **It does not sort.** `audit::sort_advisories` already ordered them — package, then whether
 *    anything fixes it, then id — and a second ordering in the frontend is the drift `pins::suggest`
 *    exists in Rust to prevent.
 * 2. **It does not show a severity.** pip-audit has no such field under either vulnerability
 *    service, so PRD P1-1's "severity-sorted" describes something that has never existed. Deriving
 *    one from the id's year or the prose would be a number PipDock invented, shown beside real ones.
 */

import { useTranslation } from 'react-i18next'

import { PdBadge } from '@/components/PdBadge'
import { PdEmptyState } from '@/components/PdEmptyState'
import { useOpenExternal } from '@/components/useOpenExternal'
import type { Advisory, AuditReport } from '@/ipc'

/**
 * How many advisory rows to render.
 *
 * The `RUFF_ROWS_SHOWN` pattern, for the same reason and with the same guarantee: **the counts
 * always come from the report**, so a capped list can never misreport a total. Two hundred rather
 * than ruff's five hundred because each row here carries a description paragraph.
 */
export const ADVISORY_ROWS_SHOWN = 200

interface PdAuditReportProps {
  report: AuditReport
  /** Overridable so a test need not build two hundred rows. */
  rowsShown?: number
}

/** One advisory. */
function AdvisoryRow({ advisory }: { advisory: Advisory }) {
  const { t } = useTranslation()
  const { open, failed } = useOpenExternal()
  const fixed = advisory.fixVersions ?? []

  return (
    <li className="rounded-pd border-l-2 border-warn bg-surface p-3">
      <div className="flex flex-wrap items-baseline gap-2">
        <code className="font-mono text-data">{advisory.id}</code>
        {fixed.length === 0 ? (
          // The row worth reading the description for: nothing to upgrade to. Stated rather than
          // left blank, which would read as "no information" instead of "no fix".
          <PdBadge tone="danger" label={t('security.noFix')} />
        ) : (
          <PdBadge tone="info" label={t('security.fixedIn', { versions: fixed.join(', ') })} />
        )}
        {advisory.url === undefined || advisory.url === null ? null : (
          <button
            type="button"
            className="text-data text-accent underline"
            onClick={() => {
              open(advisory.url ?? '')
            }}
          >
            {t('security.osv')}
          </button>
        )}
      </div>

      {(advisory.aliases ?? []).length === 0 ? null : (
        // The CVE lives here, never in `id` — PRD P1-1 says "known CVEs" and pip-audit's primary
        // id is a PYSEC under the default service.
        <p className="mt-1 font-mono text-data text-text-dim">{(advisory.aliases ?? []).join(', ')}</p>
      )}

      {advisory.description === '' ? null : (
        <p className="mt-1 text-text-dim">{advisory.description}</p>
      )}

      {failed ? (
        // A rejected `opener:allow-open-url` resolves to nothing happening. Silence is the failure
        // mode SECURITY §4 exists to name, so it is said out loud on the row that tried.
        <p role="alert" className="mt-1 text-data text-danger">
          {t('security.linkFailed')}
        </p>
      ) : null}
    </li>
  )
}

export function PdAuditReport({ report, rowsShown = ADVISORY_ROWS_SHOWN }: PdAuditReportProps) {
  const { t } = useTranslation()
  // Optional on the wire because `#[serde(default)]` makes it so; never absent in practice.
  const advisories = report.advisories ?? []

  if (advisories.length === 0) {
    // Only reachable with a report in hand, so this really is "nothing was found" rather than
    // "nothing has run" — the distinction P4 shipped the wrong side of once. The screen owns the
    // other case and never routes it here.
    return (
      <PdEmptyState
        message={t('security.clean', { count: report.packages })}
        hint={t('security.cleanHint', { version: report.toolVersion })}
      />
    )
  }

  const shown = advisories.slice(0, rowsShown)
  const remaining = advisories.length - shown.length
  const packages = new Set(advisories.map((a) => a.pkg)).size

  // Grouped by package as they arrive: core already sorted by package, so a group boundary is just
  // a change of name. Building a map here would re-impose an order on rows that already have one.
  const groups: { pkg: string; version: string; rows: Advisory[] }[] = []
  for (const advisory of shown) {
    const last = groups.at(-1)
    if (last?.pkg === advisory.pkg) last.rows.push(advisory)
    else groups.push({ pkg: advisory.pkg, version: advisory.version, rows: [advisory] })
  }

  return (
    <section aria-label={t('nav.security')}>
      <p className="text-text-dim">
        {t('security.summary', {
          count: advisories.length,
          packages,
          total: report.packages,
        })}
      </p>

      {groups.map((group) => (
        <div key={group.pkg} className="mt-4" data-pkg={group.pkg}>
          <h2 className="font-mono text-data">
            {`${group.pkg} ${group.version}`}
            <span className="ml-2 text-text-dim">{`(${String(group.rows.length)})`}</span>
          </h2>
          <ul className="mt-1 space-y-1">
            {group.rows.map((advisory) => (
              <AdvisoryRow key={`${advisory.pkg}-${advisory.id}`} advisory={advisory} />
            ))}
          </ul>
        </div>
      ))}

      {remaining > 0 ? (
        <p className="mt-4 text-text-dim">{t('security.more', { count: remaining })}</p>
      ) : null}
    </section>
  )
}
