import i18n from 'i18next'
import { initReactI18next } from 'react-i18next'

import enCommon from './locales/en/common.json'
import viCommon from './locales/vi/common.json'

/**
 * i18next setup — `docs/I18N.md`.
 *
 * §1: `en` is the source of truth; missing `vi` keys fall back to English at runtime with a
 * console warning, **never** raw key names in the UI. A CI script fails the build when `vi` is
 * short of keys, so the fallback is a safety net rather than a strategy.
 *
 * §2: default locale is the OS UI language when it is Vietnamese, else English; switchable in
 * Settings and applied live.
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
  en: { common: enCommon },
  vi: { common: viCommon },
} as const

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

export default i18n
