/**
 * The join, the dimming rule and the pinned exclusion — the pure half of TESTING §2's L3
 * obligation for `PdPackageTable`.
 *
 * Fed from `ui/src/test/fixtures/*.json`, which are **serialized from the real Rust types** by
 * `cargo run -p xtask -- ipc-fixtures` and guarded by a staleness test on the Rust side. That is
 * what makes these assertions meaningful: hand-written mock JSON would keep passing after a field
 * was renamed, against a shape the app never sends.
 */

import { describe, expect, it } from 'vitest'

import type { Dist, OutdatedDist, Pin } from '@/ipc'
import {
  formatBytes,
  joinRows,
  outdatedOnly,
  rowState,
  selectableForUpdate,
  type PackageRow,
} from '@/screens/rows'
import pinFixture from '@/test/fixtures/pin_list.json'
import listFixture from '@/test/fixtures/pkg_list.json'
import outdatedFixture from '@/test/fixtures/pkg_outdated.json'

const DISTS = listFixture as Dist[]
const OUTDATED = outdatedFixture as OutdatedDist[]
const PINS = pinFixture as Pin[]

const row = (name: string, rows: PackageRow[]): PackageRow => {
  const found = rows.find((r) => r.name === name)
  if (found === undefined) throw new Error(`fixture has no row for ${name}`)
  return found
}

describe('joinRows', () => {
  it('keeps pkg_list order and one row per installed distribution', () => {
    const { rows } = joinRows(DISTS, OUTDATED, PINS)
    expect(rows).toHaveLength(DISTS.length)
    expect(rows.map((r) => r.name)).toEqual(DISTS.map((d) => d.name))
  })

  it('fills latest only for packages in the outdated set', () => {
    const { rows } = joinRows(DISTS, OUTDATED, PINS)
    expect(row('numpy', rows).latest).toBe('2.5.1')
    // certifi is installed and current, so it has no latest at all — not the same as latest
    // equal to version, which would badge it.
    expect(row('certifi', rows).latest).toBeUndefined()
  })

  it('carries a pin through, and distinguishes the two modes', () => {
    const { rows } = joinRows(DISTS, OUTDATED, PINS)
    expect(row('scipy', rows).pin).toBe('exclude')
    expect(row('numpy', rows).pin).toEqual({ hold: { version: '1.26.4' } })
    expect(row('pandas', rows).pin).toBeUndefined()
  })

  it('leaves sizeBytes absent when the probe could not know it', () => {
    const { rows } = joinRows(DISTS, OUTDATED, PINS)
    // The editable row: RECORD lists its import shim, so the probe reports nothing rather
    // than a few hundred bytes.
    expect(row('editable-lib', rows).sizeBytes).toBeUndefined()
    expect(row('numpy', rows).sizeBytes).toBe(62_914_560)
  })

  it('reports an outdated package that has no installed row rather than dropping it', () => {
    // Really happens: the probe runs isolated and hides user-site packages, `pip list
    // --outdated` does not (SP-6). Dropping it silently would make the sidebar count promise
    // a row the table cannot show.
    const extra: OutdatedDist = { name: 'ghost', current: '1.0', latest: '2.0' }
    const { rows, orphanOutdated } = joinRows(DISTS, [...OUTDATED, extra], PINS)
    expect(orphanOutdated).toEqual(['ghost'])
    expect(rows.some((r) => r.name === 'ghost')).toBe(false)
  })
})

describe('rowState', () => {
  const { rows } = joinRows(DISTS, OUTDATED, PINS)

  it('never claims a package is up to date before the outdated set has arrived', () => {
    // The flash-of-wrong-state case: pkg_list is local and fast, pkg_outdated hits the
    // network. Reporting `current` in that window would dim every row and un-dim a handful a
    // second later — and for that second it is not true.
    for (const state of ['idle', 'loading', 'error'] as const) {
      expect(rows.map((r) => rowState(r, state))).toEqual(rows.map(() => 'unknown'))
    }
  })

  it('dims what is current and badges what is outdated, once ready', () => {
    expect(rowState(row('certifi', rows), 'ready')).toBe('current')
    expect(rowState(row('numpy', rows), 'ready')).toBe('outdated')
  })
})

describe('selectableForUpdate', () => {
  it('excludes pinned rows and reports how many, per UI-SPEC §4', () => {
    const { rows } = joinRows(DISTS, OUTDATED, PINS)
    const { selectable, pinnedExcluded } = selectableForUpdate(rows)

    // numpy (held) and scipy (excluded) are both outdated and both pinned.
    expect(pinnedExcluded).toBe(2)
    expect(selectable.toSorted()).toEqual(['pandas', 'requests'])
    expect(selectable).not.toContain('numpy')
  })

  it('never offers an up-to-date package for update', () => {
    const { rows } = joinRows(DISTS, OUTDATED, PINS)
    expect(selectableForUpdate(rows).selectable).not.toContain('certifi')
  })
})

describe('outdatedOnly', () => {
  it('is the Updates tab, and keeps pinned-but-outdated rows visible', () => {
    const { rows } = joinRows(DISTS, OUTDATED, PINS)
    const updates = outdatedOnly(rows)
    expect(updates.map((r) => r.name).toSorted()).toEqual(OUTDATED.map((o) => o.name).toSorted())
    // Visible but not selectable: hiding a pinned package would leave the user unable to see
    // why the count and the list disagree.
    expect(updates.some((r) => r.name === 'scipy')).toBe(true)
  })
})

describe('formatBytes', () => {
  it('uses binary units, per I18N §2', () => {
    expect(formatBytes(512, 'en')).toBe('512 B')
    expect(formatBytes(1024, 'en')).toBe('1.0 KiB')
    expect(formatBytes(62_914_560, 'en')).toBe('60.0 MiB')
  })

  it('formats the number in the active locale', () => {
    // I18N §2: "binary (MiB) with locale decimal separators". Vietnamese uses a comma; the
    // unit itself is data and is never translated.
    expect(formatBytes(1536, 'vi')).toBe('1,5 KiB')
    expect(formatBytes(1536, 'en')).toBe('1.5 KiB')
  })
})
