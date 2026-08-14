/**
 * The inline error row — ERROR-CATALOG §3.
 *
 * Shape is fixed: `code · localized one-liner · [Details ⌄ stderr tail] · [Copy full log] ·
 * [Report bug]`. The **code is never localized** (I18N §2) and the one-liner is looked up *from*
 * the code, so an unknown code degrades to a generic sentence rather than to a blank row or a raw
 * key. Every `Code::ALL` variant has copy in both locales as of S7, asserted by `i18n.test.ts`.
 *
 * *Copy full log* and *Report bug* both read `report_bug_url`, which is one call returning both
 * halves — the truncated excerpt inside the URL and the complete buffer for the clipboard (§4.3).
 * Neither sends anything: the URL opens in the user's browser with the issue form prefilled, and
 * they submit it or do not. That is the whole of PipDock's telemetry story.
 *
 * The two buttons are rendered only when there is a log to offer. A *Copy full log* that copies
 * nothing is worse than no button — it reports success over an empty clipboard.
 */

import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { useOpenExternal } from '@/components/useOpenExternal'
import { reportBugUrl, type BugReportLink, type PdError } from '@/ipc'
import { useUiStore } from '@/stores'

export function PdErrorRow({ error, counted = true }: { error: PdError; counted?: boolean }) {
  const { t } = useTranslation()
  const { open: openExternal, failed: openFailed } = useOpenExternal()
  const [open, setOpen] = useState(false)
  const [link, setLink] = useState<BugReportLink | null>(null)
  const [copied, setCopied] = useState(false)
  const addErrorRow = useUiStore((s) => s.addErrorRow)
  const removeErrorRow = useUiStore((s) => s.removeErrorRow)

  // Registers while mounted, so `⚠ n` means "problems currently on screen" rather than a tally
  // that only grows. `counted={false}` is for a sheet that renders one row per failed package —
  // one failed run is one problem, not forty-seven.
  useEffect(() => {
    if (!counted) return undefined
    addErrorRow()
    return removeErrorRow
  }, [counted, addErrorRow, removeErrorRow])

  // Asked for once per row rather than per click, so *Copy full log* can hide itself when there is
  // nothing to copy. A failure here costs the two buttons and nothing else — an error row that
  // errored while offering to report an error is not worth a second error row.
  useEffect(() => {
    let alive = true
    void reportBugUrl(undefined, error.code)
      .then((l) => {
        if (alive) setLink(l)
      })
      .catch(() => undefined)
    return () => {
      alive = false
    }
  }, [error.code])

  // `errors.<code>` when the catalog has it, otherwise the generic line. Falling back to the raw
  // developer message would leak English into a VI build.
  const key = `errors.${error.code}`
  const message = t(key)
  const oneLiner = message === key ? t('errors.unknown') : message

  return (
    <div role="alert" className="rounded-pd border border-danger/40 bg-surface-2 p-3">
      <p className="flex flex-wrap items-baseline gap-2">
        <code className="font-mono text-data text-danger">{error.code}</code>
        <span>{oneLiner}</span>
      </p>

      {error.stderrTail !== undefined && error.stderrTail !== '' ? (
        <>
          <button
            type="button"
            onClick={() => {
              setOpen((v) => !v)
            }}
            aria-expanded={open}
            className="mt-2 text-data text-text-dim underline underline-offset-2"
          >
            {t('actions.details')}
          </button>
          {open ? (
            <pre className="mt-2 max-h-48 overflow-auto rounded-pd bg-bg p-2 font-mono text-data text-text-dim">
              {error.stderrTail}
            </pre>
          ) : null}
        </>
      ) : null}

      {link === null ? null : (
        <div className="mt-2 flex flex-wrap items-center gap-3">
          {link.log === '' ? null : (
            <button
              type="button"
              onClick={() => {
                void writeText(link.log).then(() => {
                  setCopied(true)
                })
              }}
              data-action="copy-log"
              className="text-data text-text-dim underline underline-offset-2"
            >
              {copied ? t('errors.logCopied') : t('actions.copyLog')}
            </button>
          )}
          <button
            type="button"
            onClick={() => {
              openExternal(link.url)
            }}
            data-action="report-bug"
            className="text-data text-text-dim underline underline-offset-2"
          >
            {t('actions.reportBug')}
          </button>
          {/* §4.4: nothing is sent automatically, and the row says so rather than leaving the
              user to guess what "Report bug" does. */}
          <span className="text-data text-text-dim">{t('errors.logPrivacy')}</span>
          {openFailed ? (
            <span className="text-data text-warn" role="alert">
              {t('actions.openFailed')}
            </span>
          ) : null}
        </div>
      )}
    </div>
  )
}
