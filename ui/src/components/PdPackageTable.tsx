/**
 * The virtualized package table — UI-SPEC §4 and §6.
 *
 * One table serves both screens: Installed shows every row, Updates the same rows filtered to
 * outdated ("Same table filtered to outdated"; DATA-FLOW §4 calls them "mirrored into the Updates
 * tab"). Both read the same joined array from the store, so a package cannot be outdated in one
 * tab and not the other.
 *
 * Rows are absolutely positioned inside a spacer, so this cannot be a `<table>`. It is an ARIA
 * grid instead, and `aria-rowcount` carries the **real** total — without it a screen reader would
 * announce "row 3 of 25" in a 200-package environment, 25 being however many happen to be
 * rendered.
 */

import { useVirtualizer } from '@tanstack/react-virtual'
import { useRef } from 'react'
import { useTranslation } from 'react-i18next'

import { PdPackageRow, ROW_HEIGHT } from '@/components/PdPackageRow'
import { COLUMNS } from '@/components/packageColumns'
import type { LoadState, PackageRow } from '@/screens/rows'

interface PdPackageTableProps {
  rows: readonly PackageRow[]
  outdatedStatus: LoadState
  selection: ReadonlySet<string>
  onToggle: (name: string) => void
  onPinToggle: (name: string) => void
  onUninstall: (name: string) => void
  /** Open the dependency view focused on a package — PRD P1-6. */
  onDetails: (name: string) => void
  /**
   * Select every selectable row in the **current filtered set**, not just the rendered window.
   *
   * UI-SPEC §8 says "select-all-visible", which has no meaning once the list is virtualized:
   * "visible" would be whatever the scroll position happens to be. Resolved in S2 as the filtered
   * set, because that is what the user can see they asked for.
   */
  onSelectAll: () => void
  /**
   * jsdom reports every element as zero-height, so the virtualizer renders **zero rows** and a
   * component test passes vacuously. Tests pass a rect; production leaves it undefined.
   */
  initialRect?: { width: number; height: number }
}

export function PdPackageTable({
  rows,
  outdatedStatus,
  selection,
  onToggle,
  onPinToggle,
  onUninstall,
  onDetails,
  onSelectAll,
  initialRect,
}: PdPackageTableProps) {
  const { t } = useTranslation()
  const scrollRef = useRef<HTMLDivElement>(null)

  // React Compiler skips memoizing this component, because `useVirtualizer` returns functions it
  // cannot memoize safely. That is fine and is where the memoization actually needs to be anyway:
  // the 200-row cost is in the rows, and `PdPackageRow` is `memo`'d. Nothing from the virtualizer
  // is handed to it — only `item.index` and `item.start`, which are numbers.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    // Exact, not an estimate: one line of mono data at a fixed height, so dynamic measurement —
    // the main performance trap in a virtualizer — is never engaged.
    estimateSize: () => ROW_HEIGHT,
    overscan: 8,
    ...(initialRect === undefined ? {} : { initialRect }),
  })

  return (
    <div
      role="grid"
      aria-label={t('packages.tableLabel')}
      aria-rowcount={rows.length + 1}
      // Focusable but not a tab stop: the rows are (Sonar S6852, the ARIA grid pattern).
      tabIndex={-1}
      // `@container`: the columns size against the table's width, not the window's (U22).
      className="@container flex min-h-0 flex-1 flex-col"
      onKeyDown={(e) => {
        // Ctrl+A selects all (UI-SPEC §8). Guarded against text inputs: none exist on these
        // screens yet, but Search lands in S4 and this is the cheap moment to get it right.
        const target = e.target as HTMLElement
        const typing = target.tagName === 'INPUT' && target.getAttribute('type') !== 'checkbox'
        if (e.ctrlKey && !e.altKey && !e.metaKey && e.key.toLowerCase() === 'a' && !typing) {
          e.preventDefault()
          onSelectAll()
        }
      }}
    >
      {/* The same `COLUMNS` as every row, so the two cannot drift (U23). Outside the scroller, so
          it stays put — and given the scroller's gutter, so a scrollbar appearing does not push
          every cell left of its heading: `overflow-hidden` plus a stable gutter reserves the same
          width a scrollbar would take. */}
      <div
        role="row"
        aria-rowindex={1}
        className={`${COLUMNS.row} shrink-0 overflow-hidden border-b border-border py-1 text-data text-text-dim [scrollbar-gutter:stable]`}
      >
        {/* Aligns with the row checkbox, which has no header of its own. */}
        <span aria-hidden="true" className={COLUMNS.checkbox} />
        <span role="columnheader" className={COLUMNS.name}>
          {t('packages.column.name')}
        </span>
        <span role="columnheader" className={COLUMNS.version}>
          {t('packages.column.version')}
        </span>
        <span role="columnheader" className={COLUMNS.latest}>
          {t('packages.column.latest')}
        </span>
        <span role="columnheader" className={COLUMNS.size}>
          {t('packages.column.size')}
        </span>
        <span role="columnheader" className={COLUMNS.status}>
          {t('packages.column.status')}
        </span>
        <span aria-hidden="true" className={COLUMNS.actions} />
      </div>

      <div ref={scrollRef} className="min-h-0 flex-1 overflow-auto [scrollbar-gutter:stable]">
        <div style={{ height: virtualizer.getTotalSize(), position: 'relative' }}>
          {virtualizer.getVirtualItems().map((item) => {
            const row = rows[item.index]
            if (row === undefined) return null
            return (
              <PdPackageRow
                key={row.name}
                row={row}
                // +2: aria-rowindex is 1-based and row 1 is the header.
                ariaRowIndex={item.index + 2}
                outdatedStatus={outdatedStatus}
                selected={selection.has(row.name)}
                onToggle={onToggle}
                onPinToggle={onPinToggle}
                onUninstall={onUninstall}
                onDetails={onDetails}
                style={{
                  height: `${String(ROW_HEIGHT)}px`,
                  transform: `translateY(${String(item.start)}px)`,
                }}
              />
            )
          })}
        </div>
      </div>
    </div>
  )
}
