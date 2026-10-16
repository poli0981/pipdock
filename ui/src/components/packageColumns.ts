/**
 * The package table's column widths — one definition for the header and for every row (U23).
 *
 * The header used to be a hand-written mirror of the row, and it had drifted: one 2rem slot stood
 * above three action buttons, so every fixed column's heading sat about 76px to the right of its
 * cells. And the fixed widths summed to more than the table has at the 900px minimum window, with
 * the name column — `min-w-0 flex-1` — absorbing the whole deficit, so at that width it rendered
 * no names at all (U22). Narrow tables therefore drop Size first and tighten the rest, by
 * container query against the table's own width rather than the window's: the plan panel and a
 * future split view change the one without the other.
 *
 * Tailwind finds these by scanning source text, so every class is written out whole.
 */
export const COLUMNS = {
  row: 'flex items-center gap-2 px-3 @4xl:gap-3',
  checkbox: 'w-[13px] shrink-0',
  name: 'min-w-0 flex-1 truncate',
  version: 'w-24 shrink-0 @4xl:w-28',
  latest: 'w-24 shrink-0 @4xl:w-28',
  size: 'hidden w-24 shrink-0 text-right @3xl:block',
  status: 'flex w-36 min-w-0 shrink-0 items-center gap-1 overflow-hidden @4xl:w-40',
  actions: 'flex w-[6.5rem] shrink-0 items-center justify-end gap-1',
} as const
