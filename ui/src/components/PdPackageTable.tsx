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
import type { LoadState, PackageRow } from '@/screens/rows'

interface PdPackageTableProps {
  rows: readonly PackageRow[]
  outdatedStatus: LoadState
  selection: ReadonlySet<string>
  onToggle: (name: string) => void
  onPinToggle: (name: string) => void
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
      className="flex min-h-0 flex-1 flex-col"
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
      <div
        role="row"
        aria-rowindex={1}
        className="flex shrink-0 items-center gap-3 border-b border-border px-3 py-1 text-data text-text-dim"
      >
        {/* Aligns with the row checkbox, which has no header of its own. */}
        <span aria-hidden="true" className="w-[13px] shrink-0" />
        <span role="columnheader" className="min-w-0 flex-1">
          {t('packages.column.name')}
        </span>
        <span role="columnheader" className="w-28 shrink-0">
          {t('packages.column.version')}
        </span>
        <span role="columnheader" className="w-28 shrink-0">
          {t('packages.column.latest')}
        </span>
        <span role="columnheader" className="w-24 shrink-0 text-right">
          {t('packages.column.size')}
        </span>
        <span role="columnheader" className="w-40 shrink-0">
          {t('packages.column.status')}
        </span>
        <span aria-hidden="true" className="w-8 shrink-0" />
      </div>

      <div ref={scrollRef} className="min-h-0 flex-1 overflow-auto">
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
