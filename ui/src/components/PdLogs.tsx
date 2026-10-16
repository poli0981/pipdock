/**
 * Where PipDock's own log is, and a way to hand its recent lines to someone — Settings.
 *
 * The log files arrived in 1.4.0 (`data\logs\`, kept 14 days by the sweep, never sent anywhere).
 * There is no *Clear*: the sweep already keeps no more than that, and the files are the one record a
 * bug report needs. The folder path is shown as data — a path is never translated (I18N §2) — and
 * *Copy recent log* puts the tail on the clipboard, which `clipboard-manager:allow-write-text`
 * already grants for *Copy full log*; no capability is widened for it.
 */

import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { logsTail, type LogTail } from '@/ipc'

export function PdLogs() {
  const { t } = useTranslation()
  const [tail, setTail] = useState<LogTail | null>(null)
  const [copied, setCopied] = useState(false)

  // Once on mount, like the cache figures beside it: the log only grows while something runs, and
  // a copy re-reads it anyway.
  useEffect(() => {
    let alive = true
    void logsTail().then(
      (next) => {
        if (alive) setTail(next)
      },
      () => {
        // `logs_tail` never fails by contract; a rejection here is a bridge failure, and the
        // row simply stays without a path rather than showing an error for a convenience.
      },
    )
    return () => {
      alive = false
    }
  }, [])

  const copy = async () => {
    // Read again at the moment of copying, so the clipboard holds what the log says *now* —
    // typically the run that just went wrong — not what it said when the screen opened.
    const fresh = await logsTail()
    setTail(fresh)
    await writeText(fresh.lines.join('\n'))
    setCopied(true)
  }

  return (
    <section aria-labelledby="logs-title" className="mt-8">
      <h2 id="logs-title" className="text-text-dim">
        {t('settings.logs')}
      </h2>
      <p className="mt-1 max-w-2xl text-data text-text-dim">{t('settings.logsDetail')}</p>
      {tail !== null ? (
        <p className="mt-2 break-all font-mono text-data">{tail.dir}</p>
      ) : null}
      <div className="mt-2 flex items-center gap-3">
        <button
          type="button"
          disabled={tail === null}
          onClick={() => {
            void copy()
          }}
          className="rounded-pd border border-border px-3 py-1 text-data hover:border-accent disabled:opacity-50"
        >
          {t('settings.logsCopy')}
        </button>
        <span aria-live="polite" className="text-data text-text-dim">
          {copied ? t('settings.logsCopied') : null}
          {tail !== null && tail.lines.length === 0 && !copied ? t('settings.logsEmpty') : null}
        </span>
      </div>
    </section>
  )
}
