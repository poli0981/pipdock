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
import { PdPinChip } from '@/components/PdPinChip'
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
  onUninstall: (name: string) => void
  /** Open the dependency view focused on this package — UI-SPEC §4's "details" (PRD P1-6). */
  onDetails: (name: string) => void
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
  onUninstall,
  onDetails,
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
        // Only when the row itself has focus. Without the target check, Space on a focused action
        // button toggles the row's selection *as well as* pressing the button — and the pin
        // button was a tab stop, so that was reachable rather than theoretical.
        const onRow = e.target === e.currentTarget
        if (e.key === ' ' && !pinned && onRow) {
          // Otherwise the scroll container pages down under the focused row.
          e.preventDefault()
          onToggle(row.name)
        }
        // UI-SPEC §8's `Enter` primary action. Pin, not uninstall: a row's two actions are one
        // reversible annotation and one irreversible removal, and a destructive operation one
        // keypress from a focused row is not a primary action, it is an accident waiting.
        if (e.key === 'Enter' && onRow) {
          e.preventDefault()
          onPinToggle(row.name)
        }
        // **The ARIA grid pattern's roving tabindex, and the reason it is here at all.** Every
        // control in this row is `tabIndex={-1}` so that 200 rows do not become 600 tab stops —
        // but that left *pin* and *uninstall* reachable by mouse only, which is a WCAG 2.1.1
        // failure and breaks §8's "full keyboard traversal". Found by tabbing to a row in the
        // running app and discovering there was nowhere further to go. Arrow keys move within the
        // row; `Escape` comes back out to it.
        if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
          const controls = [
            ...e.currentTarget.querySelectorAll<HTMLElement>('button, input'),
          ].filter((el) => !(el as HTMLButtonElement).disabled)
          if (controls.length === 0) return
          e.preventDefault()
          const at = controls.indexOf(document.activeElement as HTMLElement)
          const step = e.key === 'ArrowRight' ? 1 : -1
          // From the row itself, ArrowRight enters at the first control and ArrowLeft at the last.
          const next = at === -1 ? (step === 1 ? 0 : controls.length - 1) : at + step
          if (next < 0 || next >= controls.length) {
            e.currentTarget.focus()
            return
          }
          controls[next]?.focus()
        }
        if (e.key === 'Escape' && !onRow) {
          e.preventDefault()
          e.currentTarget.focus()
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
        {row.pin === undefined ? null : <PdPinChip mode={row.pin} />}
      </span>

      {/* UI-SPEC §5's 3-click uninstall: this is the first click, *Remove* in the dialog is the
          third.

          §4 used to say the third action would fold all three into a `⋮` menu. **It does not**,
          and that is a deliberate correction rather than an oversight. A menu spends one click
          opening and one choosing, so uninstall would become Installed → ⋮ → ✕ → *Remove*: four
          clicks where §5's table records three, hand-counted, for the second most destructive
          thing in the app. The ceiling is 5 and 4 would still be under it, which is exactly why
          the budget alone would have let this through.

          A third inline button costs nothing instead. The row is the only tab stop (see
          onKeyDown), so the controls are a roving sequence rather than tab stops — a third one
          adds a left/right step, not 200 more stops. Ordered non-destructive first: details, pin,
          remove. */}
      <button
        type="button"
        tabIndex={-1}
        onClick={() => {
          onDetails(row.name)
        }}
        aria-label={t('packages.actions.details')}
        title={t('packages.actions.details')}
        className="shrink-0 rounded-pd border border-border px-2 py-0.5 text-data text-text-dim"
      >
        {'⇄'}
      </button>
      <button
        type="button"
        tabIndex={-1}
        onClick={() => {
          onPinToggle(row.name)
        }}
        aria-label={pinned ? t('packages.actions.unpin') : t('packages.actions.pin')}
        title={pinned ? t('packages.actions.unpin') : t('packages.actions.pin')}
        className="shrink-0 rounded-pd border border-border px-2 py-0.5 text-data text-text-dim"
      >
        {pinned ? '–' : '+'}
      </button>
      <button
        type="button"
        tabIndex={-1}
        onClick={() => {
          onUninstall(row.name)
        }}
        data-action="uninstall"
        aria-label={t('packages.actions.uninstall')}
        title={t('packages.actions.uninstall')}
        className="shrink-0 rounded-pd border border-border px-2 py-0.5 text-data text-text-dim"
      >
        {'✕'}
      </button>
    </div>
  )
}

export const PdPackageRow = memo(Row)
