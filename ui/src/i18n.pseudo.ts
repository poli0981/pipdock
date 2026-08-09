/**
 * The pseudo-locale — I18N §5, "to catch clipping".
 *
 * Vietnamese runs longer than English almost everywhere, and the place that shows is a button or a
 * chip sized to its English label. A pseudo-locale makes that visible without waiting for a
 * translator: every string grows ~40% and gains brackets, so a container that cannot hold it is
 * obvious at a glance rather than on the one screen someone happened to open.
 *
 * **Dev-only, and not a supported locale.** It is registered under `import.meta.env.DEV` and left
 * out of `SUPPORTED_LOCALES`, so Settings never offers it and no build ships it. `Ctrl+Alt+P`
 * toggles it, keyed on `e.code` because AltGr layouts do not agree on `e.key`.
 */

/** Padding characters, chosen to be wide and obviously not English. */
const PAD = 'ẍẍẍẍẍẍẍẍẍẍẍẍẍẍẍẍẍẍẍẍ'

/**
 * Expand one string, bracketing it so a truncation is unmistakable.
 *
 * The original text is left **exactly** as it is and the padding is appended, rather than the
 * letters being substituted the way some pseudo-locales do. Two reasons: `{{count}}` placeholders
 * survive untouched, so nothing fails for a reason that has nothing to do with layout; and the
 * string stays readable, so whoever is looking at a clipped button can still tell which one it is.
 *
 * The closing `⟧` is the tell. If it is missing, the container clipped.
 */
export function pseudoize(value: string): string {
  const grow = Math.min(Math.ceil(value.length * 0.4), PAD.length)
  return `⟦${value}${PAD.slice(0, grow)}⟧`
}

/** Recursively pseudoize a catalog. */
export function pseudoCatalog(node: unknown): unknown {
  if (typeof node === 'string') return pseudoize(node)
  if (typeof node !== 'object' || node === null) return node
  return Object.fromEntries(Object.entries(node).map(([k, v]) => [k, pseudoCatalog(v)]))
}
