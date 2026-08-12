import { describe, expect, it } from 'vitest'
import { FALLBACK_LOCALE, resolveLocale, resources, SUPPORTED_LOCALES } from './i18n'
import enCommon from './locales/en/common.json'
import enErrors from './locales/en/errors.json'
import viCommon from './locales/vi/common.json'
import viErrors from './locales/vi/errors.json'
import codes from './test/fixtures/codes.json'

/** `Code::ALL`, generated from the Rust enum — see `crates/pipdock-core/src/fixtures.rs`. */
const CODES: readonly string[] = codes

/** docs/I18N.md §4: an error one-liner has to fit the inline row. */
const MAX_ONE_LINER = 90

/** Collect every leaf key path in a nested catalog, e.g. `nav.updates`. */
function keyPaths(obj: unknown, prefix = ''): string[] {
  if (typeof obj !== 'object' || obj === null) return [prefix]
  return Object.entries(obj).flatMap(([k, v]) => keyPaths(v, prefix ? `${prefix}.${k}` : k))
}

/**
 * Drop i18next's plural suffix, so `status.packages_one` and `status.packages_other` compare as
 * one key.
 *
 * docs/I18N.md §1: **Vietnamese has a single plural form.** A vi catalog therefore carries only
 * `_other`, and comparing raw key paths would report that correct catalog as incomplete — which
 * would push whoever hit it into adding a bogus `_one` just to silence the test.
 */
function pluralBase(key: string): string {
  return key.replace(/_(zero|one|two|few|many|other)$/, '')
}

describe('the two-file merge', () => {
  /**
   * Every key from **both** source files must survive into the shipped catalog.
   *
   * The bug this exists for: `common.json` and `errors.json` both have a top-level `errors`
   * object — the codes in one, the row's furniture in the other — so a shallow
   * `{ ...common, ...errors }` dropped five strings per locale, `errors.unknown` among them. That
   * is `PdErrorRow`'s fallback for an unrecognized code, so the one thing standing between the
   * user and raw English developer text rendered as the literal `errors.unknown`.
   *
   * **Neither existing test could see it.** The catalog test only asks about `Code::ALL`, and the
   * parity test compares en against vi — which agreed perfectly, because both lost the same keys.
   * Only comparing the merge against its own inputs catches a merge that discards one.
   */
  const SOURCES = { en: [enCommon, enErrors], vi: [viCommon, viErrors] } as const

  for (const locale of SUPPORTED_LOCALES) {
    it(`${locale} keeps every key from both source files`, () => {
      const shipped = new Set(keyPaths(resources[locale].common))
      const missing = SOURCES[locale]
        .flatMap((file) => keyPaths(file))
        .filter((key) => !shipped.has(key))

      expect(missing, `lost in the merge: ${missing.join(', ')}`).toEqual([])
    })
  }
})

describe('the error catalog', () => {
  // The gap S7 exists to close: 26 of the 32 codes had no copy at all, so a PEP 668 refusal, an
  // MSVC-missing build and a disk-full all rendered as "An unexpected error occurred." with a code
  // beside them. `PdErrorRow` falls back deliberately rather than leaking `PdError.message`, which
  // is English developer text — so a missing key is silent by design, and only this catches it.
  for (const locale of SUPPORTED_LOCALES) {
    it(`${locale} has a one-liner for every catalog code`, () => {
      const table = (resources[locale].common as { errors: Record<string, string> }).errors
      const missing = CODES.filter((c) => typeof table[c] !== 'string')
      expect(missing, `${locale} is missing copy for: ${missing.join(', ')}`).toEqual([])
    })

    it(`${locale} keeps every one-liner inside the row`, () => {
      const table = (resources[locale].common as { errors: Record<string, string> }).errors
      const tooLong = CODES.filter((c) => (table[c]?.length ?? 0) > MAX_ONE_LINER).map(
        (c) => `${c} (${String(table[c]?.length)})`,
      )
      expect(tooLong, `over ${String(MAX_ONE_LINER)} chars: ${tooLong.join(', ')}`).toEqual([])
    })
  }

  it('has no copy for a code that no longer exists', () => {
    // The other direction, and the one that rots quietly: a code removed from `Code::ALL` leaves
    // copy behind that no screen can ever render.
    const table = (resources.en.common as { errors: Record<string, string> }).errors
    const orphans = Object.keys(table).filter((k) => k.startsWith('PD-') && !CODES.includes(k))
    expect(orphans, `copy for codes that do not exist: ${orphans.join(', ')}`).toEqual([])
  })
})

describe('i18n catalogs', () => {
  it('vi has every key en has', () => {
    // docs/I18N.md §1: a CI script fails the build if vi is missing keys. This is that check.
    const en = new Set([...keyPaths(resources.en.common)].map(pluralBase))
    const vi = new Set([...keyPaths(resources.vi.common)].map(pluralBase))
    const missing = [...en].filter((k) => !vi.has(k))
    expect(missing, `vi is missing: ${missing.join(', ')}`).toEqual([])
  })

  it('vi has no keys en lacks', () => {
    const en = new Set([...keyPaths(resources.en.common)].map(pluralBase))
    const extra = [...new Set([...keyPaths(resources.vi.common)].map(pluralBase))].filter(
      (k) => !en.has(k),
    )
    expect(extra, `vi has orphans: ${extra.join(', ')}`).toEqual([])
  })

  it('a plural key always has an _other form in every locale', () => {
    // `_other` is the form every language needs; `_one` is optional. A catalog with only `_one`
    // renders the key name on any count but 1, which is the failure I18N §1 forbids outright.
    for (const locale of SUPPORTED_LOCALES) {
      const keys = keyPaths(resources[locale].common)
      const plurals = new Set(keys.filter((k) => /_(zero|one|two|few|many|other)$/.test(k)))
      for (const key of plurals) {
        expect(
          plurals.has(`${pluralBase(key)}_other`),
          `${locale}: ${pluralBase(key)} has no _other form`,
        ).toBe(true)
      }
    }
  })

  it('the product name is never translated', () => {
    // docs/I18N.md §2: package names, versions, engine output and codes are never translated,
    // and the eslint allowlist treats "PipDock" as a literal for the same reason.
    expect(resources.vi.common.app.name).toBe(resources.en.common.app.name)
  })

  it.each(SUPPORTED_LOCALES)('%s catalog is non-empty', (locale) => {
    expect(keyPaths(resources[locale].common).length).toBeGreaterThan(0)
  })

  it('resolves OS language tags to a supported locale', () => {
    expect(resolveLocale('vi-VN')).toBe('vi')
    expect(resolveLocale('vi')).toBe('vi')
    expect(resolveLocale('en-GB')).toBe('en')
    // Anything else falls back to English rather than showing keys (docs/I18N.md §2).
    expect(resolveLocale('ja-JP')).toBe(FALLBACK_LOCALE)
    expect(resolveLocale(undefined)).toBe(FALLBACK_LOCALE)
    expect(resolveLocale('')).toBe(FALLBACK_LOCALE)
  })
})
