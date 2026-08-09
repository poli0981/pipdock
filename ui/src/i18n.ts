import i18n from 'i18next'
import { initReactI18next } from 'react-i18next'

import { pseudoCatalog } from './i18n.pseudo'

import enCommon from './locales/en/common.json'
import enErrors from './locales/en/errors.json'
import viCommon from './locales/vi/common.json'
import viErrors from './locales/vi/errors.json'

/**
 * i18next setup — `docs/I18N.md`.
 *
 * §1: `en` is the source of truth; missing `vi` keys fall back to English at runtime with a
 * console warning, **never** raw key names in the UI. A CI script fails the build when `vi` is
 * short of keys, so the fallback is a safety net rather than a strategy.
 *
 * §2: default locale is the OS UI language when it is Vietnamese, else English; switchable in
 * Settings and applied live.
 *
 * # One namespace, two files
 *
 * §1 lists seven namespaces. The app ships **one**, `common`, and merges the catalogs into it at
 * init. A real second namespace would mean `t('errors:PD-NET-001')` at every call site, because
 * i18next's `nsSeparator` is `:` — churn across every screen, in exchange for lazy loading that a
 * desktop app with both catalogs already in the bundle cannot use.
 *
 * The error codes get their own *file* regardless, and that part is worth it: there are 32 of them,
 * they are the only keys whose names are a contract with Rust rather than with a screen, and
 * `i18n.test.ts` checks them against `Code::ALL` itself. Splitting the file keeps that reviewable
 * without splitting the namespace. §1 is amended to say so.
 */

export const SUPPORTED_LOCALES = ['en', 'vi'] as const
export type Locale = (typeof SUPPORTED_LOCALES)[number]

export const FALLBACK_LOCALE: Locale = 'en'

/** Resolve a browser/OS language tag such as `vi-VN` to a supported locale. */
export function resolveLocale(languageTag: string | undefined): Locale {
  const base = (languageTag ?? '').split('-')[0]?.toLowerCase()
  return SUPPORTED_LOCALES.includes(base as Locale) ? (base as Locale) : FALLBACK_LOCALE
}

export const resources = {
  // Merged, not nested: `errors.json` is `{ "errors": { … } }`, so the keys land at
  // `errors.PD-XXX-NNN` exactly where `common.json` used to hold them and no call site moves.
  en: { common: { ...enCommon, ...enErrors } },
  vi: { common: { ...viCommon, ...viErrors } },
} as const

/**
 * I18N §5's pseudo-locale, in development only.
 *
 * Registered as a resource but deliberately **not** in `SUPPORTED_LOCALES`, so Settings never
 * offers it, `resolveLocale` never returns it, and the i18n parity test never compares against it.
 * `import.meta.env.DEV` keeps it out of a production bundle entirely.
 */
export const PSEUDO_LOCALE = 'en-XA'

void i18n.use(initReactI18next).init({
  resources,
  lng: resolveLocale(typeof navigator === 'undefined' ? undefined : navigator.language),
  fallbackLng: FALLBACK_LOCALE,
  defaultNS: 'common',
  interpolation: {
    // React already escapes; double-escaping mangles Vietnamese diacritics in some browsers.
    escapeValue: false,
  },
  // Surfaces the fallback in development instead of letting untranslated copy pass unnoticed.
  saveMissing: false,
  missingKeyHandler: (lngs, ns, key) => {
    console.warn(`[i18n] missing key ${ns}:${key} for ${lngs.join(',')}`)
  },
})

if (import.meta.env.DEV) {
  i18n.addResourceBundle(
    PSEUDO_LOCALE,
    'common',
    pseudoCatalog(resources.en.common),
    true,
    true,
  )
  // `e.code`, not `e.key`: on an AltGr layout the character produced by Alt+P is not "p".
  window.addEventListener('keydown', (e) => {
    if (e.ctrlKey && e.altKey && e.code === 'KeyP') {
      e.preventDefault()
      void i18n.changeLanguage(i18n.language === PSEUDO_LOCALE ? FALLBACK_LOCALE : PSEUDO_LOCALE)
    }
  })
}

export default i18n
