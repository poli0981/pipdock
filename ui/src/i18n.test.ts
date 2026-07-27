import { describe, expect, it } from 'vitest'
import { FALLBACK_LOCALE, resolveLocale, resources, SUPPORTED_LOCALES } from './i18n'

/** Collect every leaf key path in a nested catalog, e.g. `nav.updates`. */
function keyPaths(obj: unknown, prefix = ''): string[] {
  if (typeof obj !== 'object' || obj === null) return [prefix]
  return Object.entries(obj).flatMap(([k, v]) => keyPaths(v, prefix ? `${prefix}.${k}` : k))
}

describe('i18n catalogs', () => {
  it('vi has every key en has', () => {
    // docs/I18N.md §1: a CI script fails the build if vi is missing keys. This is that check.
    const en = new Set(keyPaths(resources.en.common))
    const vi = new Set(keyPaths(resources.vi.common))
    const missing = [...en].filter((k) => !vi.has(k))
    expect(missing, `vi is missing: ${missing.join(', ')}`).toEqual([])
  })

  it('vi has no keys en lacks', () => {
    const en = new Set(keyPaths(resources.en.common))
    const extra = [...new Set(keyPaths(resources.vi.common))].filter((k) => !en.has(k))
    expect(extra, `vi has orphans: ${extra.join(', ')}`).toEqual([])
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
