/**
 * The snapshot timeline — UI-SPEC §4, "timeline of snapshots with trigger label".
 *
 * The trigger label is not decoration. `update --all` writes a `Plan` snapshot before mutating and
 * a rollback writes a `Rollback` one before restoring, so a single restore moves `latest` **twice**
 * — the trap that made TESTING L2's first run report every package as changed when the rollback
 * had in fact worked. Showing which is which is what stops a user picking the wrong entry, and it
 * is why nothing in the UI ever names `latest`.
 */

import { useTranslation } from 'react-i18next'

import type { SnapshotMeta } from '@/ipc'

interface PdSnapshotTimelineProps {
  snapshots: readonly SnapshotMeta[]
  selected: string | null
  onSelect: (id: string) => void
  /** False when the environment's interpreter is gone: it can be listed but not compared. */
  selectable: boolean
}

export function PdSnapshotTimeline({
  snapshots,
  selected,
  onSelect,
  selectable,
}: PdSnapshotTimelineProps) {
  const { t, i18n } = useTranslation()

  /** The trigger, as a label. `restoring` is an id and is never translated. */
  const triggerLabel = (meta: SnapshotMeta): string => {
    if (meta.trigger === 'manual') return t('snapshots.trigger.manual')
    if ('plan' in meta.trigger) return t('snapshots.trigger.plan')
    return t('snapshots.trigger.rollback', { id: meta.trigger.rollback.restoring })
  }

  return (
    <ul className="mt-3 space-y-1">
      {snapshots.map((meta) => {
        const isSelected = meta.id === selected
        return (
          <li key={meta.id}>
            <button
              type="button"
              disabled={!selectable}
              onClick={() => {
                onSelect(meta.id)
              }}
              data-snapshot={meta.id}
              data-selected={isSelected}
              aria-current={isSelected}
              className={`flex w-full flex-wrap items-baseline gap-3 rounded-pd border px-3 py-2 text-left disabled:opacity-40 ${
                isSelected ? 'border-accent bg-surface-2' : 'border-border bg-surface'
              }`}
            >
              {/* The id is the handle every command takes, so it is shown verbatim and in mono. */}
              <code className="font-mono text-data">{meta.id}</code>
              <span className="text-data text-text-dim">
                {/* Dates are localized, ids are not (I18N §2). An unparseable timestamp falls back
                    to the raw string rather than rendering "Invalid Date". */}
                {formatWhen(meta.createdAt, i18n.language)}
              </span>
              <span className="text-data text-text-dim">
                {t('status.packages', { count: meta.packageCount })}
              </span>
              <span className="text-data text-info">{triggerLabel(meta)}</span>
            </button>
          </li>
        )
      })}
    </ul>
  )
}

function formatWhen(iso: string, locale: string): string {
  const at = new Date(iso)
  if (Number.isNaN(at.getTime())) return iso
  return new Intl.DateTimeFormat(locale, { dateStyle: 'medium', timeStyle: 'short' }).format(at)
}
