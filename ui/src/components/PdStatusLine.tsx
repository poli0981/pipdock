import { useTranslation } from 'react-i18next'

import { useEnvStore, usePlanStore, useSettingsStore, useUiStore } from '@/stores'

/**
 * The status line — UI-SPEC §3.
 *
 * Always shows env · python · engine · state in monospace, plus the log-drawer toggle. Paths,
 * versions and engine ids are data and are never translated (I18N §2); only the state word is.
 */
export function PdStatusLine() {
  const { t } = useTranslation()
  const engine = useSettingsStore((s) => s.engine)
  const rows = useEnvStore((s) => s.rows)
  const selected = useEnvStore((s) => s.selected)
  const scanning = useEnvStore((s) => s.scanning)
  const errorRows = useUiStore((s) => s.errorRows)
  const consoleOpen = usePlanStore((s) => s.consoleOpen)
  const setConsoleOpen = usePlanStore((s) => s.setConsoleOpen)
  const planPhase = usePlanStore((s) => s.phase)

  const row = rows.find((r) => r.interpreter === selected)
  // The env's own directory name is what a user calls it (".venv", "sp5"), not the whole prefix.
  // Both separators, because a prefix can arrive with either on Windows.
  const name =
    row === undefined ? null : (row.env?.prefix.split(/[\\/]/).pop() ?? row.interpreter)

  return (
    <footer className="flex items-center gap-3 border-t border-border bg-surface px-4 py-1.5 font-mono text-data text-text-dim">
      <span aria-hidden="true">{'▸'}</span>
      <span>{name ?? t('status.noEnvironment')}</span>
      {row?.env === undefined ? null : (
        <>
          <span aria-hidden="true">{'·'}</span>
          <span>{`Py ${row.env.pythonVersion}`}</span>
        </>
      )}
      <span aria-hidden="true">{'·'}</span>
      <span className="text-accent">{engine}</span>
      <span aria-hidden="true">{'·'}</span>
      <span aria-live="polite">{scanning ? t('status.scanning') : t('status.idle')}</span>

      {/* §3 draws a log toggle here and it has been plain text since S1 — a control that looks
          like one and does nothing. It opens the console drawer, which is the only log surface
          that exists until M3's logging subsystem; disabled when there is no run to show. */}
      <button
        type="button"
        onClick={() => {
          setConsoleOpen(!consoleOpen)
        }}
        disabled={planPhase === 'idle'}
        aria-expanded={consoleOpen}
        data-action="log"
        className="ml-auto underline underline-offset-2 disabled:no-underline disabled:opacity-40"
      >
        {t('status.log')}
      </button>

      {/* §3's `⚠ n`, defined at last: error rows currently on screen. */}
      <span
        data-warnings={errorRows}
        title={t('status.warningsDetail')}
        className={errorRows > 0 ? 'text-warn' : undefined}
      >
        {`⚠ ${String(errorRows)}`}
      </span>
    </footer>
  )
}
