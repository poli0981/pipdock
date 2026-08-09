/**
 * What happened — DATA-FLOW §6's summary model, rendered.
 *
 * The headline is the owner requirement: **"13 successful, 2 failed, 1 skipped"**, with an
 * expandable row per failure carrying its catalog code and stderr tail (ERROR-CATALOG §3). The
 * counts come from `ExecutionSummary.counts`, which core derives from the rows rather than
 * accumulating — a headline that disagrees with the list below it is the failure TESTING §1.4
 * says must never regress, and re-deriving it here would reintroduce exactly that risk.
 *
 * The cancelled case had no specified copy — deferred from Stage 1 with the note that killing pip
 * mid-install can leave site-packages partially written, and the summary should say so and point
 * at the snapshot. It does both.
 */

import { useTranslation } from 'react-i18next'

import { PdBadge } from '@/components/PdBadge'
import { PdErrorRow } from '@/components/PdErrorRow'
import type { ExecutionOutcome, StepResult } from '@/ipc'

interface PdSummarySheetProps {
  outcome: ExecutionOutcome
  onDone: () => void
  /**
   * Restore the snapshot this run took, when the caller can.
   *
   * Optional, and the button is absent without it — the component stays presentational and does
   * not reach for a store to find out whether a rollback is possible.
   */
  onRollback?: (id: string) => void
}

function ResultRow({ result }: { result: StepResult }) {
  const { t } = useTranslation()
  const failed = result.status === 'failed'

  return (
    <li data-status={result.status} className="rounded-pd bg-surface p-2">
      <div className="flex flex-wrap items-baseline gap-2">
        <code className="font-mono text-data">{result.pkg}</code>
        {result.to == null ? null : (
          <span className="font-mono text-data text-text-dim">
            {result.from == null ? result.to : `${result.from} → ${result.to}`}
          </span>
        )}
        <PdBadge
          tone={failed ? 'danger' : result.status === 'skipped' ? 'dim' : 'accent'}
          label={t(`plan.status.${result.status}`)}
        />
      </div>

      {/* A failure gets the full error row: code, localized one-liner, stderr tail. */}
      {failed && result.code != null ? (
        <div className="mt-2">
          <PdErrorRow
            // One failed run is one problem in the status line, not one per package: a batch
            // where 47 failed would otherwise read `⚠ 47`.
            counted={false}
            error={{
              code: result.code,
              message: result.pkg,
              ...(result.stderrTail == null ? {} : { stderrTail: result.stderrTail }),
            }}
          />
        </div>
      ) : null}
    </li>
  )
}

export function PdSummarySheet({ outcome, onDone, onRollback }: PdSummarySheetProps) {
  const { t } = useTranslation()
  const { summary, snapshot } = outcome
  const results = summary.results ?? []
  const findings = summary.check.findings ?? []

  return (
    <section aria-labelledby="summary-title" className="space-y-4">
      <div>
        <h1 id="summary-title" className="text-accent">
          {t('plan.summaryTitle')}
        </h1>
        <p aria-live="polite" className="mt-1 text-data">
          {t('plan.counts', {
            ok: summary.counts.ok,
            failed: summary.counts.failed,
            skipped: summary.counts.skipped,
          })}
        </p>
      </div>

      {/* The cancelled banner. Says the environment may be part-way, because it may be — and
          points at the snapshot, which is the only complete answer to that. */}
      {summary.cancelled === true ? (
        <p className="rounded-pd border-l-2 border-warn bg-warn/10 p-2 text-data text-warn">
          {t('plan.cancelledDetail')}
        </p>
      ) : null}

      {snapshot === undefined ? null : (
        <div className="flex flex-wrap items-baseline gap-2">
          <p className="text-data text-text-dim">
            {/* The id is data, never translated — it is what `pipdock snapshot rollback` takes. */}
            {t('plan.snapshotTaken')} <code className="font-mono">{snapshot.id}</code>
          </p>
          {/* The cancelled banner has promised since S3 that "the snapshot below restores the
              environment exactly as it was", with nothing behind it. This is the button that
              sentence has been describing. */}
          {onRollback === undefined ? null : (
            <button
              type="button"
              onClick={() => {
                onRollback(snapshot.id)
              }}
              data-action="rollback"
              className="rounded-pd border border-danger px-2 py-0.5 text-data text-danger"
            >
              {t('snapshots.rollbackThis')}
            </button>
          )}
        </div>
      )}

      {/* Post-run `pip check`. A finding here means the environment is inconsistent *after* a run
          that may have reported every step ok, so it is not folded into the counts. */}
      {findings.length > 0 ? (
        <div className="rounded-pd border-l-2 border-warn p-2">
          <p className="text-data text-warn">{t('plan.checkFailed', { count: findings.length })}</p>
          <ul className="mt-1 space-y-0.5">
            {findings.map((f) => (
              <li key={`${f.pkg}-${f.requirement}`} className="font-mono text-data text-text-dim">
                {t('plan.checkFinding', { pkg: f.pkg, requirement: f.requirement })}
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      <ul className="space-y-2">
        {results.map((r) => (
          <ResultRow key={r.pkg} result={r} />
        ))}
      </ul>

      <button
        type="button"
        onClick={onDone}
        className="rounded-pd border border-border px-3 py-1 text-data"
      >
        {t('plan.done')}
      </button>
    </section>
  )
}
