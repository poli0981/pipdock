/**
 * Ranking a short list of actions against what the user typed — the command palette's matcher.
 *
 * **Deliberately not `index_search`.** That command searches 864,000 PyPI *package names* over
 * IPC, which is the wrong corpus and the wrong cost for thirty actions; a palette that made a
 * round trip per keystroke would be slower than the thing it exists to speed up. `nucleo` is a
 * `pipdock-core` dependency and there is no JS fuzzy library, so this is twenty lines instead.
 *
 * What it *does* take from `crates/pipdock-core/src/index/mod.rs` is the ranking philosophy, which
 * was learned the hard way in SP-3: **tiered, not score-ordered.** A raw fuzzy score rewards
 * density, so a longer name containing the query densely beats the name the user actually typed —
 * nucleo ranks `requests-ntlm` above `requests`. Exact, then prefix, then subsequence; within a
 * tier, the shorter label; then alphabetical, so two identical queries never reorder.
 */

/** How a candidate matched, in ranking order. Lower sorts first. */
export const enum MatchKind {
  Exact = 0,
  Prefix = 1,
  Subsequence = 2,
}

/**
 * Is `needle` a subsequence of `haystack`?
 *
 * A two-pointer walk, which is the part that is easy to get wrong: `[...needle].every(c =>
 * haystack.includes(c))` tests *presence*, not order, and would match `slot` against `tools`.
 * Both are lower-cased by the caller.
 */
export function isSubsequence(needle: string, haystack: string): boolean {
  let at = 0
  for (const c of haystack) {
    if (c === needle[at]) at += 1
    if (at === needle.length) return true
  }
  return at === needle.length
}

/** How `haystack` matched `needle`, or null when it did not. */
export function matchKind(needle: string, haystack: string): MatchKind | null {
  if (haystack === needle) return MatchKind.Exact
  if (haystack.startsWith(needle)) return MatchKind.Prefix
  return isSubsequence(needle, haystack) ? MatchKind.Subsequence : null
}

/**
 * How much worse a match on the secondary text is than any match on the primary.
 *
 * Larger than the widest tier gap, so **every** label match outranks **every** group match. Found
 * by a test: with one flat text of `"${group} ${label}"`, typing `search` put *Download the index*
 * — group "Search", so a prefix match — above the Search tab itself, whose label matched only as a
 * subsequence of `go to search`. Someone typing a screen's name wants the screen.
 */
const SECONDARY = 10

/**
 * Rank `items` against `query`, keeping only what matches.
 *
 * `text` returns the primary label and, optionally, a secondary the item may also be found by —
 * its group. A match on the primary always beats a match on the secondary, whatever the tiers.
 *
 * An empty query keeps everything in its original order — a palette that opens showing nothing is
 * a palette that has to be taught before it can be used.
 */
export function rank<T>(
  items: readonly T[],
  query: string,
  text: (item: T) => readonly [primary: string, secondary?: string],
): T[] {
  const needle = query.trim().toLowerCase()
  if (needle === '') return [...items]

  return items
    .map((item) => {
      const [primary, secondary] = text(item)
      const label = primary.toLowerCase()
      const direct = matchKind(needle, label)
      if (direct !== null) return { item, label, score: direct }
      const via = secondary === undefined ? null : matchKind(needle, secondary.toLowerCase())
      return via === null ? null : { item, label, score: via + SECONDARY }
    })
    .filter((r): r is { item: T; label: string; score: number } => r !== null)
    .sort(
      (a, b) =>
        a.score - b.score ||
        // The shorter label, because a short one containing the query is nearly always the thing
        // itself and the longer ones are its variants.
        a.label.length - b.label.length ||
        a.label.localeCompare(b.label),
    )
    .map((r) => r.item)
}
