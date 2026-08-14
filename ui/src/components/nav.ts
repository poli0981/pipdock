/**
 * Sidebar tab order — UI-SPEC §3 and §8.
 *
 * The order is load-bearing: `Ctrl+1..9` maps to it positionally, so reordering these entries
 * silently rebinds the user's shortcuts.
 *
 * **Appending is free; inserting is not.** `about` went on the end in Phase 4 and became `Ctrl+9`
 * without moving a single existing binding — `App.tsx` reads `NAV_KEYS[key - 1]`, so a ninth entry
 * costs nothing but a ninth label. Anything placed *before* `settings` would shift every shortcut
 * after it, which is why Snapshots is a mode of Environments rather than an entry of its own: it
 * would have to sit beside Environments to read as related, and that is an insert.
 */
export const NAV_KEYS = [
  'environments',
  'installed',
  'updates',
  'search',
  'pins',
  'health',
  'security',
  'settings',
  'about',
] as const

export type NavKey = (typeof NAV_KEYS)[number]
