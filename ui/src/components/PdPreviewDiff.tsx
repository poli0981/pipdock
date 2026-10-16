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
import type { Blocker, Change, ChangeKind, Decision, ResolutionReport } from '@/ipc'

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

/** A dependent's version, when the probe knew it — composed as `PdConflictRow` composes it. */
const dependent = (b: Blocker) => (b.version == null ? (b.by ?? '') : `${b.by ?? ''} ${b.version}`)

export function PdPreviewDiff({ report, decisions, onChoose }: PdPreviewDiffProps) {
  const { t } = useTranslation()
  const changes = report.changes ?? []
  const heldBack = report.heldBack ?? []
  const impossible = report.impossible ?? null

  const incompatible = report.incompatible ?? []
  const forced = report.forced ?? []
  const yanked = report.yanked ?? []
  // One per broken dependent, the same count the Confirm button carries.
  const breaks = new Set(
    forced.flatMap((f) => (f.blockers ?? []).map((b) => b.by ?? b.constraint)),
  ).size

  // A plan whose only content is a package left out for its Python is not "nothing to change":
  // the user asked for something and is owed the reason it is not happening.
  const nothingToShow =
    changes.length === 0 &&
    heldBack.length === 0 &&
    impossible === null &&
    incompatible.length === 0 &&
    forced.length === 0 &&
    yanked.length === 0

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

      {forced.length > 0 ? (
        // DISCLAIMER §2: what forcing breaks is said before it happens, naming each dependent
        // and the constraint of its that the forced version violates.
        <section aria-labelledby="preview-forced" data-section="forced">
          <h2 id="preview-forced" className="text-data text-danger">
            {breaks > 0
              ? t('plan.section.forced', { count: breaks })
              : t('plan.section.forcedOnly', { count: forced.length })}
          </h2>
          <ul className="mt-1 space-y-1">
            {forced.map((f) => (
              <li key={f.pkg} className="font-mono text-data">
                {/* Versions are data and are never localized (I18N §2). */}
                <code className="text-danger">{`${f.pkg} → ${f.latest}`}</code>
                <ul className="ml-4 text-text-dim">
                  {(f.blockers ?? []).map((b, i) => (
                    <li key={`${b.by ?? ''}-${b.constraint}-${String(i)}`}>
                      {b.by == null
                        ? b.constraint
                        : t('plan.blocker', { by: dependent(b), constraint: b.constraint })}
                    </li>
                  ))}
                </ul>
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      {incompatible.length > 0 ? (
        // Hard invariant 10: left out, with the range set against this interpreter — never the
        // "not found" pip says for a version that exists.
        <section aria-labelledby="preview-incompatible" data-section="incompatible">
          <h2 id="preview-incompatible" className="text-data text-warn">
            {t('plan.section.incompatible', { count: incompatible.length })}
          </h2>
          <ul className="mt-1 space-y-0.5">
            {incompatible.map((m) => (
              <li key={`${m.pkg}-${m.version}`} className="font-mono text-data text-text-dim">
                {t('plan.incompatibleRow', {
                  pkg: m.pkg,
                  version: m.version,
                  range: m.requiresPython,
                  python: m.python,
                })}
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      {yanked.length > 0 ? (
        // PD-PKG-003 is a warning, not a failure (SP-2): both engines install a yanked release.
        // Both parsers extracted these and the adapters dropped them, so none was ever shown.
        <section aria-labelledby="preview-yanked" data-section="yanked">
          <h2 id="preview-yanked" className="text-data text-warn">
            {t('plan.section.yanked', { count: yanked.length })}
          </h2>
          <ul className="mt-1 space-y-0.5">
            {yanked.map((y) => (
              <li key={`${y.pkg}-${y.version}`} className="font-mono text-data text-text-dim">
                {y.reason == null
                  ? t('plan.yankedRow', { pkg: y.pkg, version: y.version })
                  : t('plan.yankedReason', { pkg: y.pkg, version: y.version, reason: y.reason })}
              </li>
            ))}
          </ul>
        </section>
      ) : null}

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
