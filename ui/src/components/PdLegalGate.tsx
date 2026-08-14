/**
 * First-run legal gate — UI-SPEC §4 and PRD P0-13.
 *
 * Five documents, one checkbox, and **Decline exits the app**. Consent is stored with the
 * documents' hash, so a bump re-triggers this (`docs_hash`, computed at build time).
 *
 * The links are opened by Rust, not by the webview: `connect-src` in `tauri.conf.json` allows only
 * `'self'` and the IPC origin, and the `opener` capability is scoped to three hosts (SECURITY §4).
 * A failure to open is shown rather than swallowed — the documents are the thing the user is being
 * asked to agree to, so silently failing to show them is not acceptable. That rule is now the
 * shared `useOpenExternal`, and the rest of the app follows it too.
 *
 * **`review` is presentational and nothing else.** About re-opens this screen so the documents can
 * be read again; in that mode the checkbox and both buttons are replaced by a single Close, and no
 * consent is read or written. The prop defaults to false, so the first-run path — the one that is
 * legally load-bearing — renders exactly as it did before the prop existed, and a test pins that.
 */

import { getCurrentWindow } from '@tauri-apps/api/window'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { LEGAL_DOCUMENTS } from '@/components/legal'
import { useOpenExternal } from '@/components/useOpenExternal'
import { useLegalStore } from '@/stores'

export function PdLegalGate({
  review = false,
  onClose,
}: {
  review?: boolean
  onClose?: () => void
} = {}) {
  const { t } = useTranslation()
  const accept = useLegalStore((s) => s.accept)
  const [checked, setChecked] = useState(false)
  const { open, failed: openFailed } = useOpenExternal()

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="legal-title"
      className="fixed inset-0 z-50 flex items-center justify-center bg-bg/95 p-6"
    >
      <div className="max-h-full w-full max-w-2xl overflow-auto rounded-pd border border-border bg-surface p-6">
        <h1 id="legal-title" className="font-mono text-accent">
          {t('legal.title')}
        </h1>

        <p className="mt-3 text-text-dim">{t('legal.intro')}</p>

        <ul className="mt-3 space-y-1">
          {LEGAL_DOCUMENTS.map(({ key, href }) => (
            <li key={key}>
              <button
                type="button"
                onClick={() => {
                  open(href)
                }}
                className="text-accent-dim underline underline-offset-2 hover:text-accent"
              >
                {t(`legal.documents.${key}`)}
              </button>
            </li>
          ))}
        </ul>

        {openFailed ? (
          <p className="mt-3 text-warn" role="alert">
            {t('legal.openFailed')}
          </p>
        ) : null}

        <p className="mt-4 border-l-2 border-border pl-3 text-text-dim">{t('legal.summary')}</p>

        {review ? (
          <div className="mt-5 flex gap-2">
            <button
              type="button"
              onClick={onClose}
              className="rounded-pd bg-accent px-4 py-1.5 text-bg"
            >
              {t('legal.close')}
            </button>
          </div>
        ) : (
          <>
            <label className="mt-5 flex items-start gap-2">
              <input
                type="checkbox"
                checked={checked}
                onChange={(e) => {
                  setChecked(e.target.checked)
                }}
                className="mt-1 accent-[var(--color-accent)]"
              />
              <span>{t('legal.accept')}</span>
            </label>

            <div className="mt-5 flex gap-2">
              <button
                type="button"
                disabled={!checked}
                onClick={() => {
                  void accept()
                }}
                className="rounded-pd bg-accent px-4 py-1.5 text-bg disabled:opacity-40"
              >
                {t('actions.accept')}
              </button>
              {/* UI-SPEC §4: declining exits. Nothing here is usable without agreeing. */}
              <button
                type="button"
                onClick={() => {
                  void getCurrentWindow().close()
                }}
                className="rounded-pd border border-border px-4 py-1.5 text-text-dim"
              >
                {t('actions.decline')}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  )
}
