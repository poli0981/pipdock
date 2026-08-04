/**
 * The install queue — UI-SPEC §4's **dock bay**.
 *
 * "Docks along the right edge as a slim column of added packages with editable version fields and
 * `Install (n)`." It persists across searches on purpose: adding four packages should be four
 * searches and one install, not four installs.
 *
 * An empty version field means *latest*, which is exactly what a `PlanRequest` with no specifier
 * means — so the common case needs no typing, and the field is there for the case that does.
 */

import { useTranslation } from 'react-i18next'

import type { QueuedPackage } from '@/stores/index-search'

interface PdDockBayProps {
  queue: readonly QueuedPackage[]
  onVersionChange: (name: string, version: string) => void
  onRemove: (name: string) => void
  onInstall: () => void
  /** Disabled without a selected environment — there would be nowhere to install to. */
  canInstall: boolean
}

export function PdDockBay({
  queue,
  onVersionChange,
  onRemove,
  onInstall,
  canInstall,
}: PdDockBayProps) {
  const { t } = useTranslation()

  if (queue.length === 0) return null

  return (
    <aside
      aria-label={t('search.dockBay')}
      className="flex w-64 shrink-0 flex-col border-l border-border bg-surface"
    >
      <h2 className="shrink-0 border-b border-border px-3 py-2 text-data text-text-dim">
        {t('search.dockBay')}
      </h2>

      <ul className="min-h-0 flex-1 space-y-1 overflow-auto p-2">
        {queue.map((item) => (
          <li key={item.name} data-queued={item.name} className="rounded-pd bg-surface-2 p-2">
            <div className="flex items-baseline justify-between gap-2">
              <code className="min-w-0 truncate font-mono text-data">{item.name}</code>
              <button
                type="button"
                onClick={() => {
                  onRemove(item.name)
                }}
                aria-label={t('search.remove', { pkg: item.name })}
                className="shrink-0 text-data text-text-dim"
              >
                {'✕'}
              </button>
            </div>
            <input
              type="text"
              value={item.version}
              onChange={(e) => {
                onVersionChange(item.name, e.target.value)
              }}
              // Empty is the common case and already means latest, so the placeholder says so
              // rather than leaving the field looking unfinished.
              placeholder={t('search.latest')}
              aria-label={t('search.versionFor', { pkg: item.name })}
              className="mt-1 w-full rounded-pd border border-border bg-bg px-2 py-0.5 font-mono text-data"
            />
          </li>
        ))}
      </ul>

      <div className="shrink-0 border-t border-border p-2">
        <button
          type="button"
          onClick={onInstall}
          disabled={!canInstall}
          title={canInstall ? undefined : t('search.noEnvironment')}
          className="w-full rounded-pd border border-accent px-3 py-1 text-data text-accent disabled:border-border disabled:text-text-dim disabled:opacity-40"
        >
          {t('search.install', { count: queue.length })}
        </button>
      </div>
    </aside>
  )
}
