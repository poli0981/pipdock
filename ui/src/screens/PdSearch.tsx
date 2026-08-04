/**
 * Search — UI-SPEC §4, and the screen the < 50 ms keystroke budget belongs to.
 *
 * "Search field autofocused; results stream under 50 ms per keystroke from the local index. Result
 * row: name · summary · latest · `INSTALLED ✓`/`UPDATE` chip when applicable · **[Add]**."
 *
 * The already-installed chips are the reason this screen reads `useEnvStore`: DATA-FLOW §4 says a
 * package already present shows its state instead of an [Add] button, which stops the most common
 * mistake — queueing something you already have.
 */

import { useEffect, useMemo, useRef } from 'react'
import { useTranslation } from 'react-i18next'

import { PdBadge } from '@/components/PdBadge'
import { PdDockBay } from '@/components/PdDockBay'
import { PdEmptyState } from '@/components/PdEmptyState'
import { PdErrorRow } from '@/components/PdErrorRow'
import { useEnvStore, usePlanStore } from '@/stores'
import { queueSpecs, useIndexStore } from '@/stores/index-search'

export function PdSearch() {
  const { t } = useTranslation()
  const field = useRef<HTMLInputElement>(null)

  const query = useIndexStore((s) => s.query)
  const hits = useIndexStore((s) => s.hits)
  const ready = useIndexStore((s) => s.ready)
  const unavailable = useIndexStore((s) => s.unavailable)
  const searching = useIndexStore((s) => s.searching)
  const error = useIndexStore((s) => s.error)
  const selected = useIndexStore((s) => s.selected)
  const meta = useIndexStore((s) => s.meta)
  const metaFreshness = useIndexStore((s) => s.metaFreshness)
  const queue = useIndexStore((s) => s.queue)
  const refreshing = useIndexStore((s) => s.refreshing)

  const setQuery = useIndexStore((s) => s.setQuery)
  const select = useIndexStore((s) => s.select)
  const enqueue = useIndexStore((s) => s.enqueue)
  const setQueuedVersion = useIndexStore((s) => s.setQueuedVersion)
  const dequeue = useIndexStore((s) => s.dequeue)
  const clearQueue = useIndexStore((s) => s.clearQueue)
  const refreshIndex = useIndexStore((s) => s.refreshIndex)

  const packages = useEnvStore((s) => s.packages)
  const rows = useEnvStore((s) => s.rows)
  const envSelected = useEnvStore((s) => s.selected)
  const planResolve = usePlanStore((s) => s.resolve)

  const envRow = rows.find((r) => r.interpreter === envSelected)

  // UI-SPEC §5: the Search flow's first click is spent arriving here, so the field must already
  // have focus — "Search (autofocus) → result [Add] → Install → Confirm" is 4, not 5.
  useEffect(() => {
    field.current?.focus()
  }, [])

  // What is already installed, so a result can say so instead of offering [Add].
  const installed = useMemo(
    () => new Map(packages.map((p) => [p.name, p])),
    [packages],
  )

  return (
    <section aria-labelledby="search-title" className="flex h-full min-h-0">
      <div className="flex min-h-0 min-w-0 flex-1 flex-col p-6">
        <h1 id="search-title" className="shrink-0 text-accent">
          {t('search.title')}
        </h1>

        <input
          ref={field}
          type="search"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value)
          }}
          placeholder={t('search.placeholder')}
          aria-label={t('search.title')}
          className="mt-3 shrink-0 rounded-pd border border-border bg-surface px-3 py-1.5 font-mono text-data"
        />

        <div className="mt-1 flex shrink-0 items-center gap-2 text-data text-text-dim">
          {/* Warming is a state, not a failure: the index is 864k names and takes ~140 ms to
              load, and saying so beats a field that silently returns nothing. */}
          {!ready && unavailable === null ? <span>{t('search.warming')}</span> : null}
          {searching ? <span>{t('search.searching')}</span> : null}
        </div>

        {unavailable !== null ? (
          <div className="mt-3 shrink-0 rounded-pd border-l-2 border-warn p-2">
            <p className="text-data text-warn">{t('search.indexMissing')}</p>
            <button
              type="button"
              onClick={() => {
                void refreshIndex()
              }}
              disabled={refreshing}
              className="mt-2 rounded-pd border border-border px-3 py-1 text-data disabled:opacity-40"
            >
              {refreshing ? t('search.refreshing') : t('search.refreshIndex')}
            </button>
          </div>
        ) : null}

        {error !== null ? (
          <div className="mt-3 shrink-0">
            <PdErrorRow error={error} />
          </div>
        ) : null}

        {query.trim() !== '' && hits.length === 0 && ready && !searching ? (
          <PdEmptyState message={t('search.noResults', { query })} hint={t('search.noResultsHint')} />
        ) : null}

        <ul className="mt-3 min-h-0 flex-1 space-y-1 overflow-auto">
          {hits.map((hit) => {
            const have = installed.get(hit.name)
            const queued = queue.some((q) => q.name === hit.name)
            return (
              <li
                key={hit.name}
                data-hit={hit.name}
                data-match={hit.kind}
                className={`flex items-center gap-2 rounded-pd p-2 ${
                  selected === hit.name ? 'bg-surface-2' : 'bg-surface'
                }`}
              >
                <button
                  type="button"
                  onClick={() => {
                    void select(hit.name)
                  }}
                  className="min-w-0 flex-1 text-left"
                >
                  {/* The display name as PyPI spells it — data, never localized (I18N §2). */}
                  <code className="font-mono text-data">{hit.display}</code>
                </button>

                {/* DATA-FLOW §4: an installed package shows its state rather than [Add]. */}
                {have === undefined ? null : (
                  <PdBadge
                    tone={have.latest === undefined ? 'accent' : 'warn'}
                    label={
                      have.latest === undefined
                        ? t('search.installed')
                        : t('packages.badge.update')
                    }
                  />
                )}

                {have === undefined ? (
                  <button
                    type="button"
                    onClick={() => {
                      enqueue(hit.name)
                    }}
                    disabled={queued}
                    className="shrink-0 rounded-pd border border-border px-2 py-0.5 text-data disabled:opacity-40"
                  >
                    {queued ? t('search.queued') : t('search.add')}
                  </button>
                ) : null}
              </li>
            )
          })}
        </ul>

        {/* The detail panel. `stale` is not an error — ARCHITECTURE §5 keeps showing cached
            metadata offline rather than failing, and says which it is. */}
        {meta !== null ? (
          <div className="mt-3 shrink-0 rounded-pd border-t border-border pt-3">
            <div className="flex items-baseline gap-2">
              <code className="font-mono text-data">{meta.name}</code>
              {meta.version == null ? null : (
                <span className="font-mono text-data text-text-dim">{meta.version}</span>
              )}
              {metaFreshness === 'fresh' ? null : (
                <PdBadge tone="dim" label={t(`search.freshness.${metaFreshness ?? 'cached'}`)} />
              )}
            </div>
            {meta.summary == null ? null : <p className="mt-1 text-data">{meta.summary}</p>}
            <p className="mt-1 text-data text-text-dim">
              {meta.requiresPython == null
                ? ''
                : t('search.requiresPython', { spec: meta.requiresPython })}
              {meta.license == null ? '' : ` · ${meta.license}`}
            </p>
          </div>
        ) : null}
      </div>

      <PdDockBay
        queue={queue}
        onVersionChange={setQueuedVersion}
        onRemove={dequeue}
        canInstall={envRow?.env !== undefined}
        onInstall={() => {
          if (envRow?.env === undefined) return
          const specs = queueSpecs(queue)
          clearQueue()
          void planResolve(envRow.env, { intent: 'install', specs })
        }}
      />
    </section>
  )
}
