/**
 * When pip itself needs upgrading (PRD P0-10).
 *
 * Pure and React-free for the same reason `rows.ts` is: the rule is worth testing without
 * rendering anything.
 */

/**
 * The version below which pip cannot produce `--dry-run --report` output.
 *
 * Mirrors `engine::PIP_MIN_VERSION_FOR_REPORT`. Both sides cite **DATA-FLOW §7** ("pip ≥ 22.2 for
 * `--dry-run --report`; offer in-app pip upgrade when older") rather than each other, because
 * nothing generates this across the bridge — it is two numbers in a document, and the document is
 * the source. If it ever moves, it moves in the doc first.
 */
export const PIP_MIN_FOR_REPORT: readonly [number, number] = [22, 2]

/**
 * Whether this environment's pip is too old to plan with.
 *
 * **The only condition that surfaces the row's *Upgrade pip* button.** Deliberately not "or a newer
 * pip exists": that needs `pkg_outdated`, which is networked and per-environment, so it would put N
 * network calls on the landing screen to surface something the Installed and Updates screens
 * already offer. What those screens *cannot* do is upgrade a pip so old that the planner behind
 * them refuses to run — which is exactly this case, and why the button earns its place.
 *
 * `undefined` is not "old": a probe that found no pip has nothing to upgrade, and offering to
 * would produce `PD-ENG-001` on click.
 */
export function pipNeedsUpgrade(version: string | undefined): boolean {
  const parsed = parseRelease(version)
  if (parsed === null) return false

  const [major, minor] = parsed
  const [minMajor, minMinor] = PIP_MIN_FOR_REPORT
  return major < minMajor || (major === minMajor && minor < minMinor)
}

/**
 * The leading `major.minor` of a pip version, or `null` when there isn't one.
 *
 * pip's own versions are `YY.N` or `YY.N.P`, but a version string is data from the environment, so
 * anything can arrive: `23.0`, `23.0.1`, `24.1b1`, a local segment, or something not a version at
 * all. Only the two leading integers are read, and a string that does not start with them is
 * `null` — treated as "do not offer", because guessing wrong here shows a destructive-ish button
 * on an environment that did not need it.
 */
function parseRelease(version: string | undefined): [number, number] | null {
  if (version === undefined) return null

  const match = /^(\d+)\.(\d+)/.exec(version)
  if (match === null) return null

  const major = Number(match[1])
  const minor = Number(match[2])
  return Number.isFinite(major) && Number.isFinite(minor) ? [major, minor] : null
}
