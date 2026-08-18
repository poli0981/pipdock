/**
 * What PipDock has written to disk, and what can be removed — PRD P1-4.
 *
 * **Three artefacts, not four.** There are no log files: `LOG_RETENTION_DAYS` has never had a
 * reader, the only log is the in-memory ring buffer behind *Report bug*, and `logs_tail` is still
 * owed. A "logs — 0 B" row would be inventing a thing that does not exist.
 *
 * **`index.db` has no Clear button**, and its row says why. It holds the package index *and* the
 * settings *and* the pin list *and* the legal-consent record in one SQLite file, so a "clear the
 * cache" that removed it would take a user's pins with it — the same class of surprise the privacy
 * policy had to be corrected for. Its size is still shown, because a report that omitted the
 * largest thing on disk would be answering a different question than the one the user asked.
 *
 * Clearing is confirmed through `PdDialog`, which puts Cancel first and focuses it (UI-SPEC §7).
 * Removing snapshots is not reversible and the confirm says exactly what is lost.
 */

import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { PdDialog } from '@/components/PdDialog'
import { PdErrorRow } from '@/components/PdErrorRow'
import { cacheClear, cacheUsage, type CacheTarget, type CacheUsage, type PdError } from '@/ipc'
import { formatBytes } from '@/screens/rows'
import { asPdError, useSettingsStore } from '@/stores'

/** The rows that can be cleared, in the order they are offered. */
const CLEARABLE: readonly CacheTarget[] = ['snapshots', 'tools', 'audit']

export function PdCacheUsage() {
  const { t } = useTranslation()
  const locale = useSettingsStore((s) => s.locale)
  const [usage, setUsage] = useState<CacheUsage | null>(null)
  const [error, setError] = useState<PdError | null>(null)
  const [confirming, setConfirming] = useState<CacheTarget | null>(null)
  const [busy, setBusy] = useState(false)

  const load = async () => {
    try {
      setUsage(await cacheUsage())
      setError(null)
    } catch (e) {
      setError(asPdError(e))
    }
  }

  // Once on mount — the numbers only move when PipDock itself writes, and every path that does is
  // a deliberate action the user just took elsewhere. The promise chain is inline rather than a
  // call to `load` because `react-hooks/set-state-in-effect` cannot see through an async function,
  // and the same shape is what `useAppInfo` uses.
  useEffect(() => {
    let alive = true
    void cacheUsage().then(
      (u) => {
        if (alive) setUsage(u)
      },
      (e: unknown) => {
        if (alive) setError(asPdError(e))
      },
    )
    return () => {
      alive = false
    }
  }, [])

  // A lookup rather than a ternary chain: the chain's last branch was a hand-maintained negation
  // of every case above it, so adding `audit` as a third target would have silently reported the
  // tools venv's size on the audit row.
  const entryOf = (target: CacheTarget) =>
    ({ snapshots: usage?.snapshots, tools: usage?.tools, audit: usage?.audit })[target]

  const doClear = async (target: CacheTarget) => {
    setBusy(true)
    try {
      await cacheClear(target)
      setError(null)
      await load()
    } catch (e) {
      setError(asPdError(e))
    } finally {
      setBusy(false)
      setConfirming(null)
    }
  }

  return (
    <fieldset className="mt-6">
      <legend className="text-text-dim">{t('cache.title')}</legend>
      <p className="mt-1 max-w-2xl text-data text-text-dim">{t('cache.intro')}</p>

      {error !== null ? (
        <div className="mt-2">
          <PdErrorRow error={error} />
        </div>
      ) : null}

      {usage === null ? null : (
        <ul className="mt-2 max-w-2xl space-y-1">
          <li className="flex flex-wrap items-baseline gap-x-3 rounded-pd border border-border bg-surface px-3 py-1.5">
            <span className="min-w-0 flex-1 text-data">{t('cache.database')}</span>
            <code className="font-mono text-data text-text-dim">
              {formatBytes(usage.database.bytes, locale)}
            </code>
            {/* No button, and the reason is on screen rather than only in a comment. */}
            <span className="w-full text-data text-text-dim">{t('cache.databaseWhy')}</span>
          </li>

          {CLEARABLE.map((target) => {
            const entry = entryOf(target)
            if (entry === undefined) return null
            return (
              <li
                key={target}
                data-cache={target}
                className="flex flex-wrap items-baseline gap-x-3 rounded-pd border border-border bg-surface px-3 py-1.5"
              >
                <span className="min-w-0 flex-1 text-data">
                  {t(`cache.${target}`)}
                  {target === 'snapshots' && usage.snapshotCount > 0 ? (
                    <span className="ml-1 text-text-dim">
                      {t('cache.snapshotCount', { count: usage.snapshotCount })}
                    </span>
                  ) : null}
                </span>
                <code className="font-mono text-data text-text-dim">
                  {formatBytes(entry.bytes, locale)}
                </code>
                <button
                  type="button"
                  // Nothing there is not an error, so the button simply has nothing to do.
                  disabled={!entry.exists || busy}
                  onClick={() => {
                    setConfirming(target)
                  }}
                  data-action="clear-cache"
                  className="shrink-0 rounded-pd border border-border px-2 py-0.5 text-data text-text-dim disabled:opacity-40"
                >
                  {t('cache.clear')}
                </button>
              </li>
            )
          })}
        </ul>
      )}

      {confirming !== null ? (
        <PdDialog
          label={t(`cache.${confirming}`)}
          title={t('cache.confirmTitle')}
          cancelLabel={t('actions.cancel')}
          busy={busy}
          onCancel={() => {
            setConfirming(null)
          }}
          actions={
            <button
              type="button"
              disabled={busy}
              onClick={() => {
                void doClear(confirming)
              }}
              data-action="confirm-clear-cache"
              className="rounded-pd border border-danger px-3 py-1 text-data text-danger disabled:opacity-40"
            >
              {t('cache.clear')}
            </button>
          }
        >
          <p>{t(`cache.confirm.${confirming}`)}</p>
        </PdDialog>
      ) : null}
    </fieldset>
  )
}
