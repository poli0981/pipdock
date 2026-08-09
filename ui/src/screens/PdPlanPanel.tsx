/**
 * The mutation spine on screen — DATA-FLOW §3 from `Resolving` to `Summary`.
 *
 * Replaces the package table rather than sitting beside it (UI-SPEC §4): one plan, one screen, and
 * nothing to mis-click while a decision is pending.
 *
 * It renders the phase the store is in and nothing else. Which phase is allowed next is the
 * flow's business, in Rust — this never infers it, because two implementations of that state
 * machine is exactly what `core::flow` exists to prevent (G5).
 */

import { useTranslation } from 'react-i18next'

import { PdConsoleDrawer } from '@/components/PdConsoleDrawer'
import { PdErrorRow } from '@/components/PdErrorRow'
import { PdPreviewDiff } from '@/components/PdPreviewDiff'
import { PdRollbackPreview } from '@/components/PdRollbackPreview'
import { PdSummarySheet } from '@/components/PdSummarySheet'
import { usePlanStore } from '@/stores'

interface PdPlanPanelProps {
  /** Called when the user leaves the summary, so the caller can re-read the environment. */
  onFinished: () => void
}

export function PdPlanPanel({ onFinished }: PdPlanPanelProps) {
  const { t } = useTranslation()

  const phase = usePlanStore((s) => s.phase)
  const step = usePlanStore((s) => s.step)
  const kind = usePlanStore((s) => s.kind)
  const preview = usePlanStore((s) => s.preview)
  const decisions = usePlanStore((s) => s.decisions)
  const consoleLines = usePlanStore((s) => s.console)
  const consoleOpen = usePlanStore((s) => s.consoleOpen)
  const done = usePlanStore((s) => s.done)
  const total = usePlanStore((s) => s.total)
  const current = usePlanStore((s) => s.current)
  const outcome = usePlanStore((s) => s.outcome)
  const error = usePlanStore((s) => s.error)
  const cancelling = usePlanStore((s) => s.cancelling)

  const choose = usePlanStore((s) => s.choose)
  const submitDecisions = usePlanStore((s) => s.submitDecisions)
  const execute = usePlanStore((s) => s.execute)
  const cancel = usePlanStore((s) => s.cancel)
  const setConsoleOpen = usePlanStore((s) => s.setConsoleOpen)
  const reset = usePlanStore((s) => s.reset)

  // A resolve that never produced a preview. Without its own branch it falls through to the one
  // below and draws a round counter for rounds that do not exist and a Confirm for a plan that was
  // never made — over an error row explaining that there is no plan.
  if (phase === 'failed') {
    return (
      <section className="h-full overflow-auto p-6">
        <h1 className="text-accent">{t('plan.previewTitle')}</h1>
        {error === null ? null : (
          <div className="mt-4">
            <PdErrorRow error={error} />
          </div>
        )}
        <button
          type="button"
          onClick={() => {
            reset()
            onFinished()
          }}
          className="mt-4 rounded-pd border border-border px-3 py-1 text-data"
        >
          {t('actions.back')}
        </button>
      </section>
    )
  }

  if (phase === 'summary') {
    return (
      <section className="h-full overflow-auto p-6">
        {error === null ? null : (
          <div className="mb-4">
            <PdErrorRow error={error} />
          </div>
        )}
        {outcome === null ? null : (
          <PdSummarySheet
            outcome={outcome}
            onDone={() => {
              reset()
              onFinished()
            }}
          />
        )}
        {outcome === null && error === null ? null : (
          <button
            type="button"
            onClick={() => {
              reset()
              onFinished()
            }}
            className="mt-4 rounded-pd border border-border px-3 py-1 text-data"
          >
            {t('plan.done')}
          </button>
        )}
      </section>
    )
  }

  // A rollback produces no `FlowStep` — there is nothing to resolve and nothing to decide — so
  // every guard below that reads `step` would answer "no" and disable the one button the flow
  // needs. `kind` is what distinguishes "no plan" from "a plan of a different shape".
  const isRollback = kind === 'rollback'
  const rollbackOps =
    preview === null ? 0 : preview.restore.uninstall.length + preview.restore.install.length
  const changeCount = isRollback
    ? rollbackOps
    : step !== null && 'report' in step
      ? (step.report.changes ?? []).length
      : 0
  const needsDecisions = step?.step === 'needsDecisions'
  const exhausted = step?.step === 'roundsExhausted'
  const nothingToDo = isRollback ? preview !== null && rollbackOps === 0 : step?.step === 'nothing'
  const confirmable = isRollback ? preview !== null && rollbackOps > 0 : step !== null && step.step !== 'nothing'

  return (
    <section className="flex h-full min-h-0 flex-col">
      <div className="min-h-0 flex-1 overflow-auto p-6">
        <div className="flex items-baseline justify-between">
          <h1 className="text-accent">
            {isRollback ? t('snapshots.rollbackTitle') : t('plan.previewTitle')}
          </h1>
          <span aria-live="polite" className="text-data text-text-dim">
            {phase === 'resolving' ? t('plan.resolving') : null}
            {phase === 'executing'
              ? `${t('plan.executing')} ${current ?? ''} ${
                  total > 0 ? t('plan.progress', { done, total }) : ''
                }`
              : null}
          </span>
        </div>

        {error === null ? null : (
          <div className="mt-4">
            <PdErrorRow error={error} />
          </div>
        )}

        {/* The round counter UI-SPEC §4 now requires. `MAX_CONFLICT_ROUNDS` existed in core with
            nothing surfacing it, so a user could hit the cap unwarned. */}
        {needsDecisions ? (
          <p className="mt-2 text-data text-text-dim">
            {t('plan.roundsLeft', { count: step.roundsRemaining })}
          </p>
        ) : null}
        {exhausted ? <p className="mt-2 text-data text-warn">{t('plan.roundsExhausted')}</p> : null}

        {preview !== null ? (
          <div className="mt-4">
            <PdRollbackPreview preview={preview} />
          </div>
        ) : null}

        {step !== null && 'report' in step ? (
          <div className="mt-4">
            <PdPreviewDiff report={step.report} decisions={decisions} onChoose={choose} />
          </div>
        ) : null}

        {nothingToDo ? (
          <p className="mt-4 text-data text-text-dim">{t('plan.emptyPreview')}</p>
        ) : null}
      </div>

      <PdConsoleDrawer
        lines={consoleLines}
        open={consoleOpen}
        done={done}
        total={total}
        onClose={() => {
          setConsoleOpen(false)
        }}
      />

      <div className="flex shrink-0 items-center gap-2 border-t border-border p-3">
        {phase === 'executing' ? (
          <>
            <button
              type="button"
              onClick={() => {
                void cancel()
              }}
              disabled={cancelling}
              className="rounded-pd border border-danger px-3 py-1 text-data text-danger disabled:opacity-40"
            >
              {cancelling ? t('plan.stopping') : t('plan.stop')}
            </button>
            <button
              type="button"
              onClick={() => {
                setConsoleOpen(!consoleOpen)
              }}
              className="rounded-pd border border-border px-3 py-1 text-data"
            >
              {t('plan.console')}
            </button>
          </>
        ) : (
          <>
            <button
              type="button"
              onClick={() => {
                reset()
                onFinished()
              }}
              className="rounded-pd border border-border px-3 py-1 text-data"
            >
              {t('actions.back')}
            </button>
            {needsDecisions ? (
              <button
                type="button"
                onClick={() => {
                  void submitDecisions()
                }}
                disabled={phase === 'resolving'}
                className="rounded-pd border border-border px-3 py-1 text-data disabled:opacity-40"
              >
                {t('plan.applyDecisions')}
              </button>
            ) : null}
            {/* Confirm stays available while decisions are pending: the defaults are a valid
                answer, which is what makes "one conflict kept compatible" still 4 clicks. */}
            <button
              type="button"
              onClick={() => {
                void execute()
              }}
              disabled={phase === 'resolving' || !confirmable}
              className={`rounded-pd border px-3 py-1 text-data disabled:border-border disabled:text-text-dim disabled:opacity-40 ${
                // A restore removes things. It gets the danger treatment the update path does not.
                isRollback ? 'border-danger text-danger' : 'border-accent text-accent'
              }`}
            >
              {isRollback
                ? t('snapshots.confirmRollback', { count: changeCount })
                : t('plan.confirm', { count: changeCount })}
            </button>
          </>
        )}
      </div>
    </section>
  )
}
