/**
 * The client-side join behind the Installed and Updates tables.
 *
 * `pkg_list`, `pkg_outdated` and `pin_list` are three commands (ARCHITECTURE §7) and the table is
 * one view of all three. Nothing in the core performs this join — the CLI never needs it, since
 * `pipdock list` and `pipdock list --outdated` are separate commands with separate output. So it
 * lives here, as a pure function, which is what lets the dimming and badging rules be tested
 * without rendering anything (TESTING §2's L3 obligation).
 *
 * The join key is the PEP 503-normalized name. Every producer already normalizes — `parse_probe`,
 * both engine JSON parsers and `pins::list` all go through `PkgName::parse`, and since S2 so does
 * anything arriving over IPC — so plain string equality is correct here and no normalization is
 * repeated in TypeScript.
 */

import type { Dist, OutdatedDist, Pin, PinMode } from '@/ipc'

/**
 * Where a load has got to.
 *
 * `pkg_list` is local and fast; `pkg_outdated` hits the network. Between them there is a window
 * where the table is on screen and outdatedness is simply not known yet, and that is a third
 * state rather than "up to date" — see {@link rowState}.
 */
export type LoadState = 'idle' | 'loading' | 'ready' | 'error'

/** What the row's appearance is derived from. */
export type RowState = 'unknown' | 'current' | 'outdated'

/** One row of the table: the join of the three commands. */
export interface PackageRow {
  name: string
  version: string
  /** From `pkg_outdated`. Absent means "not outdated" **only** once outdated is `ready`. */
  latest?: string
  /** From `pkg_list`. Absent when the probe judged it unknowable (editables, `.egg-info`). */
  sizeBytes?: number
  /** From `pin_list`. Present ⇒ excluded from *Select all* (DATA-FLOW §9.5). */
  pin?: PinMode
}

/**
 * Join the three responses into the table's rows, in the order `pkg_list` returned them.
 *
 * An outdated entry with no installed row is dropped and reported separately: it means the two
 * sources disagree about what is installed, which really happens — `probe.py` runs with `-I` and
 * hides user-site packages, while `pip list --outdated` does not (SP-6). Silently dropping it
 * would make the sidebar's Updates count promise rows the table cannot show.
 */
export function joinRows(
  dists: readonly Dist[],
  outdated: readonly OutdatedDist[],
  pins: readonly Pin[],
): { rows: PackageRow[]; orphanOutdated: string[] } {
  const latestByName = new Map(outdated.map((o) => [o.name, o.latest]))
  const pinByName = new Map(pins.map((p) => [p.pkg, p.mode]))

  const rows = dists.map((d) => {
    const latest = latestByName.get(d.name)
    const pin = pinByName.get(d.name)
    // Conditional spread rather than `latest: undefined`: `exactOptionalPropertyTypes` is on,
    // and widening these to `T | undefined` would collapse the unknown/current distinction.
    return {
      name: d.name,
      version: d.version,
      ...(latest === undefined ? {} : { latest }),
      ...(d.sizeBytes == null ? {} : { sizeBytes: d.sizeBytes }),
      ...(pin === undefined ? {} : { pin }),
    }
  })

  const installed = new Set(dists.map((d) => d.name))
  const orphanOutdated = outdated.map((o) => o.name).filter((n) => !installed.has(n))

  return { rows, orphanOutdated }
}

/**
 * How a row should render.
 *
 * **Never dim on a guess.** While `pkg_outdated` is still in flight every row is `unknown`, so the
 * table shows them all at full strength and badges nothing. Treating "not in the outdated set" as
 * "up to date" during that window would dim all 200 rows and then un-dim a handful a second later
 * — a visible flash, and for that second it is not true.
 */
export function rowState(row: PackageRow, outdatedStatus: LoadState): RowState {
  if (outdatedStatus !== 'ready') return 'unknown'
  return row.latest === undefined ? 'current' : 'outdated'
}

/** The Updates tab: the same rows filtered to outdated (UI-SPEC §4). */
export function outdatedOnly(rows: readonly PackageRow[]): PackageRow[] {
  return rows.filter((r) => r.latest !== undefined)
}

/**
 * What *Select all* selects, and what it had to leave out.
 *
 * **This is presentation, not enforcement.** DATA-FLOW §9.5 is enforced by `pins::filter_upgrades`
 * when a plan is built, and S3's preview must report `excluded_pins()` from the flow rather than
 * this number. Duplicating the rule here is only so the button can say what it did before anything
 * is planned — UI-SPEC §4 requires "3 pinned excluded", because a selection that silently ignores
 * part of what you asked for is worse than one that refuses.
 */
export function selectableForUpdate(rows: readonly PackageRow[]): {
  selectable: string[]
  pinnedExcluded: number
} {
  const candidates = rows.filter((r) => r.latest !== undefined)
  return {
    selectable: candidates.filter((r) => r.pin === undefined).map((r) => r.name),
    pinnedExcluded: candidates.filter((r) => r.pin !== undefined).length,
  }
}

/** Binary units, per I18N §2 ("file sizes binary (MiB) with locale decimal separators"). */
const UNITS = ['B', 'KiB', 'MiB', 'GiB', 'TiB'] as const

/**
 * Render an installed size.
 *
 * `Intl` has no binary units, so the unit is chosen here and only the number goes through
 * `Intl.NumberFormat` — which is the part that is locale-dependent (Vietnamese uses a comma as the
 * decimal separator). The unit itself is data and is never translated.
 */
export function formatBytes(bytes: number, locale: string): string {
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024
    unit += 1
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1
  const formatted = new Intl.NumberFormat(locale, {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  }).format(value)
  return `${formatted} ${UNITS[unit]}`
}
