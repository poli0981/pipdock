/**
 * Sidebar tab order — UI-SPEC §3 and §8.
 *
 * The order is load-bearing: `Ctrl+1..8` maps to it positionally, so reordering these entries
 * silently rebinds the user's shortcuts.
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
] as const

export type NavKey = (typeof NAV_KEYS)[number]
