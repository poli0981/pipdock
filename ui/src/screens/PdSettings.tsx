/**
 * Settings — UI-SPEC §4.
 *
 * Engine, language and the PEP 668 override. Changes save immediately, which is what makes
 * "switch engine" a 3-click flow (UI-SPEC §5) rather than 4 with a Save button.
 *
 * The override is the one control here with teeth. SECURITY §3 requires it off by default, with
 * warning copy that does not soften the risk (I18N §4 says the same about translating it), and the
 * `--break-system-packages` equivalent is passed **only** when it is on — hard invariant 5.
 */

import { useEffect } from 'react'
import { useTranslation } from 'react-i18next'

import { PdErrorRow } from '@/components/PdErrorRow'
import { SUPPORTED_LOCALES, type Locale } from '@/i18n'
import { useSettingsStore } from '@/stores'

const ENGINES = ['pip', 'uv'] as const

export function PdSettings() {
  const { t, i18n } = useTranslation()
  const {
    engine,
    allowExternallyManaged,
    pinSuggestThreshold,
    locale,
    loading,
    error,
    load,
    save,
    setLocale,
  } = useSettingsStore()

  useEffect(() => {
    void load()
  }, [load])

  return (
    <section aria-labelledby="settings-title" className="h-full overflow-auto p-6">
      <h1 id="settings-title" className="text-accent">
        {t('settings.title')}
      </h1>

      {error !== null ? (
        <div className="mt-4">
          <PdErrorRow error={error} />
        </div>
      ) : null}

      <fieldset className="mt-6" disabled={loading}>
        <legend className="text-text-dim">{t('settings.engine')}</legend>
        <p className="mt-1 text-data text-text-dim">{t('settings.engineDetail')}</p>
        <div className="mt-2 flex gap-4">
          {ENGINES.map((id) => (
            <label key={id} className="flex items-center gap-2">
              <input
                type="radio"
                name="engine"
                checked={engine === id}
                onChange={() => {
                  void save({ engine: id })
                }}
                className="accent-[var(--color-accent)]"
              />
              {/* Engine names are product identifiers, never translated (I18N §2). */}
              <span className="font-mono text-data">{id}</span>
            </label>
          ))}
        </div>
      </fieldset>

      <fieldset className="mt-6">
        <legend className="text-text-dim">{t('settings.locale')}</legend>
        <div className="mt-2 flex gap-4">
          {SUPPORTED_LOCALES.map((code: Locale) => (
            <label key={code} className="flex items-center gap-2">
              <input
                type="radio"
                name="locale"
                checked={locale === code}
                onChange={() => {
                  setLocale(code)
                  // I18N §2: applied live, not on restart.
                  void i18n.changeLanguage(code)
                }}
                className="accent-[var(--color-accent)]"
              />
              <span className="font-mono text-data">{code}</span>
            </label>
          ))}
        </div>
      </fieldset>

      <fieldset className="mt-6" disabled={loading}>
        <legend className="text-text-dim">{t('settings.pinThreshold')}</legend>
        <p className="mt-1 max-w-2xl text-data text-text-dim">{t('settings.pinThresholdDetail')}</p>
        <input
          type="number"
          min={0}
          max={999}
          step={1}
          value={pinSuggestThreshold}
          aria-label={t('settings.pinThreshold')}
          onChange={(e) => {
            // The first number input in the app, so the rule is set here: **reject at the
            // boundary, never store junk.** `<input type="number">` reports `''` for anything it
            // cannot parse — including `abc` and a lone `-` — and coercing that would write 0,
            // which is a meaningful setting ("off") the user did not ask for.
            const next = Number.parseInt(e.target.value, 10)
            if (!Number.isFinite(next) || next < 0 || next > 999) return
            void save({ pinSuggestThreshold: next })
          }}
          className="mt-2 w-20 rounded-pd border border-border bg-bg px-2 py-0.5 font-mono text-data"
        />
      </fieldset>

      <fieldset className="mt-6" disabled={loading}>
        <legend className="text-text-dim">{t('settings.override')}</legend>
        <label className="mt-2 flex items-start gap-2">
          <input
            type="checkbox"
            checked={allowExternallyManaged}
            onChange={(e) => {
              void save({ allowExternallyManaged: e.target.checked })
            }}
            className="mt-1 accent-[var(--color-danger)]"
          />
          <span className="text-data text-warn">{t('settings.overrideDetail')}</span>
        </label>
      </fieldset>
    </section>
  )
}
