/**
 * One row of the package table — UI-SPEC §4: `name (mono) · version · latest · size · chips`.
 *
 * Memoized, which only pays off if every callback the table passes is referentially stable — take
 * them from the store as actions, never as inline arrows in the screen. With 200 rows and a
 * progress tick firing, an unstable callback re-renders the whole window on every tick.
 *
 * The row carries `data-state` and `data-pinned` **derived from the same expressions as its class
 * names**. L3 asserts on those rather than on Tailwind classes, so a restyle does not break the
 * test while a change to the dimming *rule* still does.
 */

import { memo } from 'react'
import { useTranslation } from 'react-i18next'

import { PdBadge } from '@/components/PdBadge'
import { formatBytes, rowState, type LoadState, type PackageRow } from '@/screens/rows'

/** Fixed row height, in px. Mono data at one line, so the virtualizer never has to measure. */
export const ROW_HEIGHT = 34

interface PdPackageRowProps {
  row: PackageRow
  /** 1-based and including the header, because only a window of rows is ever in the DOM. */
  ariaRowIndex: number
  outdatedStatus: LoadState
  selected: boolean
  onToggle: (name: string) => void
  onPinToggle: (name: string) => void
  /** The virtualizer's absolute positioning. */
  style: React.CSSProperties
}

function Row({
  row,
  ariaRowIndex,
  outdatedStatus,
  selected,
  onToggle,
  onPinToggle,
  style,
}: PdPackageRowProps) {
  const { t, i18n } = useTranslation()
  const state = rowState(row, outdatedStatus)
  const pinned = row.pin !== undefined

  return (
    <div
      role="row"
      aria-rowindex={ariaRowIndex}
      aria-selected={selected}
      data-state={state}
      data-pinned={pinned}
      // UI-SPEC §8: Space toggles row selection. The row itself is the tab stop, not the
      // checkbox — with 200 rows, tabbing through two controls each is not traversal, it is a
      // punishment. `tabIndex={0}` plus this handler is what makes the row the unit.
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === ' ' && !pinned) {
          // Otherwise the scroll container pages down under the focused row.
          e.preventDefault()
          onToggle(row.name)
        }
      }}
      style={style}
      className={`absolute top-0 left-0 flex w-full items-center gap-3 border-b border-border px-3 ${
        // UI-SPEC §4: up-to-date rows are dimmed. `unknown` is deliberately not dimmed —
        // see rowState.
        state === 'current' ? 'text-text-dim' : 'text-text'
      } ${selected ? 'bg-surface-2' : 'bg-surface'}`}
    >
      <input
        type="checkbox"
        checked={selected}
        // A pinned row is visible but not selectable: DATA-FLOW §9.5 keeps it out of bulk
        // updates, and showing it greyed says so more honestly than hiding it.
        disabled={pinned}
        onChange={() => {
          onToggle(row.name)
        }}
        aria-label={row.name}
        // Not a tab stop: the row is (see onKeyDown above). Still clickable and still announced.
        tabIndex={-1}
        className="shrink-0 accent-accent disabled:opacity-40"
      />

      <code role="gridcell" className="min-w-0 flex-1 truncate font-mono text-data">
        {row.name}
      </code>
      <span role="gridcell" className="w-28 shrink-0 font-mono text-data">
        {row.version}
      </span>
      <span role="gridcell" className="w-28 shrink-0 font-mono text-data">
        {/* An em dash, not "0" or a blank: latest is unknown until pkg_outdated resolves. */}
        {row.latest ?? '—'}
      </span>
      <span role="gridcell" className="w-24 shrink-0 text-right font-mono text-data">
        {row.sizeBytes === undefined ? '—' : formatBytes(row.sizeBytes, i18n.language)}
      </span>

      <span role="gridcell" className="flex w-40 shrink-0 items-center gap-1">
        {state === 'outdated' ? <PdBadge tone="warn" label={t('packages.badge.update')} /> : null}
        {row.pin === undefined ? null : (
          <PdBadge
            tone="info"
            glyph={'\u{1F512}\u{FE0E}'}
            // A Hold pin restates a version in every plan and an Exclude pin does not, so the
            // chip has to tell them apart — UI-SPEC §4 says only "a 🔒 chip".
            label={row.pin === 'exclude' ? t('packages.badge.pinned') : row.pin.hold.version}
            title={
              row.pin === 'exclude'
                ? t('packages.badge.pinnedDetail')
                : t('packages.badge.heldDetail', { version: row.pin.hold.version })
            }
          />
        )}
      </span>

      {/* UI-SPEC §5 spends the first of the 3-click uninstall budget here, so the menu exists in
          S2 even though Uninstall itself arrives with the guard dialog in S5. */}
      <button
        type="button"
        onClick={() => {
          onPinToggle(row.name)
        }}
        aria-label={pinned ? t('packages.actions.unpin') : t('packages.actions.pin')}
        title={pinned ? t('packages.actions.unpin') : t('packages.actions.pin')}
        className="shrink-0 rounded-pd border border-border px-2 py-0.5 text-data text-text-dim"
      >
        {pinned ? '–' : '+'}
      </button>
    </div>
  )
}

export const PdPackageRow = memo(Row)
