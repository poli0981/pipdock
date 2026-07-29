/**
 * The inline error row — ERROR-CATALOG §3.
 *
 * Shape is fixed: `code · localized one-liner · [Details ⌄ stderr tail] · [Copy full log]`. The
 * **code is never localized** (I18N §2) and the one-liner is looked up *from* the code, so an
 * unknown code degrades to a generic sentence rather than to a blank row or a raw key.
 */

import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import type { PdError } from '@/ipc'

export function PdErrorRow({ error }: { error: PdError }) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)

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
    </div>
  )
}
