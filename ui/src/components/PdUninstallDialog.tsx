/**
 * DATA-FLOW §5's guard dialog: the three options, and what they cost.
 *
 * ```text
 *   breaks nothing → Confirm dialog (lists removals)
 *   breaks {Y,Z}   → [Cancel] [Remove dependents too] (adds Y,Z, re-guard) [Force remove only X]
 * ```
 *
 * *Remove dependents too* **re-guards**. It is not a variant of the flow but the caller starting
 * again over a wider set, because pulling Y in can break Z — stopping after one level would hand
 * the user a set that still breaks something, and removing it anyway is the exact behaviour bare
 * `pip uninstall` has that this guard exists to fix.
 *
 * The breakage sentence is composed here, not in Rust: `GuardReport.breaks` maps the package being
 * removed to its dependents, each carrying the *bare* specifier (`"<2,>=1.26.0"`). Rust emits
 * codes and structured data only (I18N §1); the specifier, the names and the versions are data and
 * are never translated, and only the words between them are a catalog key.
 */

import { useTranslation } from 'react-i18next'

import { PdDialog } from '@/components/PdDialog'
import type { GuardReport } from '@/ipc'

interface PdUninstallDialogProps {
  report: GuardReport
  /** True while a re-guard is in flight. */
  busy: boolean
  onCancel: () => void
  /** *Remove dependents too* — re-guards over `report.withDependents`. */
  onWiden: () => void
  /** Confirm. `force` is true when the guard objected and the user chose to break things. */
  onConfirm: (force: boolean) => void
}

export function PdUninstallDialog({
  report,
  busy,
  onCancel,
  onWiden,
  onConfirm,
}: PdUninstallDialogProps) {
  const { t } = useTranslation()
  const entries = Object.entries(report.breaks)
  const clear = entries.length === 0
  const extra = report.withDependents.filter((p) => !report.removing.includes(p))
  // Distinct dependents, not rows: one package can appear under two removals, and the sentence
  // counts packages that would break rather than lines in the list below.
  const brokenCount = new Set(entries.flatMap(([, ds]) => ds.map((d) => d.pkg))).size

  return (
    <PdDialog
      label={report.removing.join(', ')}
      title={clear ? t('uninstall.title') : t('uninstall.titleBreaks')}
      cancelLabel={t('actions.cancel')}
      onCancel={onCancel}
      busy={busy}
      actions={
        clear ? (
          <button
            type="button"
            disabled={busy}
            onClick={() => {
              onConfirm(false)
            }}
            data-action="remove"
            className="rounded-pd border border-danger px-3 py-1 text-data text-danger disabled:opacity-40"
          >
            {t('uninstall.remove', { count: report.removing.length })}
          </button>
        ) : (
          <>
            {/* Offered before the destructive option, because it is the one that leaves nothing
                broken — and it re-guards, so choosing it is never the last word. */}
            <button
              type="button"
              disabled={busy || extra.length === 0}
              onClick={onWiden}
              data-action="widen"
              className="rounded-pd border border-border px-3 py-1 text-data disabled:opacity-40"
            >
              {t('uninstall.widen', { count: extra.length })}
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => {
                onConfirm(true)
              }}
              data-action="force"
              className="rounded-pd border border-danger bg-danger/20 px-3 py-1 text-data text-danger disabled:opacity-40"
            >
              {t('uninstall.force', { count: report.removing.length })}
            </button>
          </>
        )
      }
    >
      <p>
        {clear
          ? t('uninstall.body', { count: report.removing.length })
          : t('uninstall.bodyBreaks', { count: brokenCount })}
      </p>

      <ul className="mt-2 space-y-1">
        {report.removing.map((pkg) => (
          <li key={pkg} data-removing={pkg}>
            <code className="font-mono">{pkg}</code>
          </li>
        ))}
      </ul>

      {clear ? null : (
        <ul className="mt-3 space-y-1" data-breakage>
          {entries.map(([removed, dependents]) =>
            dependents.map((d) => (
              <li key={`${removed}:${d.pkg}:${d.constraint}`} className="text-warn">
                {/* `pandas 2.1.4 requires numpy<2,>=1.26.0` — the specifier is the point. A name
                    alone says what breaks and not whether the user can live with it. */}
                {t('plan.breakage', {
                  by: d.version == null ? d.pkg : `${d.pkg} ${d.version}`,
                  constraint: `${removed}${d.constraint}`,
                })}
              </li>
            )),
          )}
        </ul>
      )}

      {busy ? (
        <p aria-live="polite" className="mt-3 text-text-dim">
          {t('uninstall.rechecking')}
        </p>
      ) : null}
    </PdDialog>
  )
}
