/**
 * The resolved plan, grouped — UI-SPEC §4's preview panel, DATA-FLOW §3's `PreviewReady`.
 *
 * This is what PRD G1 promises: **every mutating operation is previewed before execution**. The
 * grouping is the point, not decoration — "what will change" and "what needs me" are different
 * questions and a flat list answers neither.
 *
 * `Will downgrade` is its own section, added with this component. `ChangeKind` has four variants
 * and UI-SPEC named three groups, so `Downgrade` had nowhere to go — and a *compatible* resolve
 * routinely moves a package down to satisfy something else. Folding it in with the upgrades would
 * put `2.0 → 1.9` under a heading reading "Will upgrade", which is misleading about exactly the
 * change most likely to surprise.
 */

import { useTranslation } from 'react-i18next'

import { PdConflictRow } from '@/components/PdConflictRow'
import { PdEmptyState } from '@/components/PdEmptyState'
import type { Change, ChangeKind, Decision, ResolutionReport } from '@/ipc'

/** Sections in the order they are read: what happens, then what is asked of you. */
const SECTIONS: readonly { kind: ChangeKind; tone: string }[] = [
  { kind: 'upgrade', tone: 'text-text' },
  // Warn, for the same reason it has its own heading.
  { kind: 'downgrade', tone: 'text-warn' },
  { kind: 'new-install', tone: 'text-text' },
  { kind: 'new-dependency', tone: 'text-text-dim' },
]

interface PdPreviewDiffProps {
  report: ResolutionReport
  decisions: Record<string, Decision>
  onChoose: (pkg: string, decision: Decision) => void
}

function ChangeRow({ change, tone }: { change: Change; tone: string }) {
  return (
    <li className={`flex items-baseline gap-2 font-mono text-data ${tone}`}>
      <code className="min-w-40">{change.name}</code>
      <span>
        {/* Versions are data and are never localized (I18N §2). */}
        {change.from == null ? change.to : `${change.from} → ${change.to}`}
      </span>
    </li>
  )
}

export function PdPreviewDiff({ report, decisions, onChoose }: PdPreviewDiffProps) {
  const { t } = useTranslation()
  const changes = report.changes ?? []
  const heldBack = report.heldBack ?? []
  const impossible = report.impossible ?? null

  const nothingToShow = changes.length === 0 && heldBack.length === 0 && impossible === null

  if (nothingToShow) {
    return <PdEmptyState message={t('plan.emptyPreview')} hint={t('plan.emptyPreviewHint')} />
  }

  return (
    <div className="space-y-4">
      {SECTIONS.map(({ kind, tone }) => {
        const rows = changes.filter((c) => c.kind === kind)
        if (rows.length === 0) return null
        return (
          <section key={kind} aria-labelledby={`preview-${kind}`} data-section={kind}>
            <h2 id={`preview-${kind}`} className="text-data text-text-dim">
              {t(`plan.section.${kind}`, { count: rows.length })}
            </h2>
            <ul className="mt-1 space-y-0.5">
              {rows.map((c) => (
                <ChangeRow key={c.name} change={c} tone={tone} />
              ))}
            </ul>
          </section>
        )
      })}

      {heldBack.length > 0 || impossible !== null ? (
        <section aria-labelledby="preview-decisions" data-section="needs-decision">
          <h2 id="preview-decisions" className="text-data text-warn">
            {t('plan.section.needsDecision', {
              count: heldBack.length + (impossible?.packages?.length ?? 0),
            })}
          </h2>
          <ul className="mt-1 space-y-2">
            {heldBack.map((h) => (
              <PdConflictRow
                key={h.pkg}
                pkg={h.pkg}
                resolved={h.resolved}
                latest={h.latest}
                blockers={h.blockers ?? []}
                impossible={false}
                value={decisions[h.pkg]}
                onChoose={(d) => {
                  onChoose(h.pkg, d)
                }}
              />
            ))}
            {(impossible?.packages ?? []).map((pkg) => (
              <PdConflictRow
                key={pkg}
                pkg={pkg}
                blockers={
                  // The resolver's own explanation, which is the only account of an impossible
                  // set there is — PipDock never re-derives resolution (ARCHITECTURE §1.2).
                  impossible === null ? [] : [{ by: null, constraint: impossible.explanation }]
                }
                impossible
                value={decisions[pkg]}
                onChoose={(d) => {
                  onChoose(pkg, d)
                }}
              />
            ))}
          </ul>
        </section>
      ) : null}
    </div>
  )
}
