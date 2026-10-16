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

import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { PdCacheUsage } from '@/components/PdCacheUsage'
import { PdDialog } from '@/components/PdDialog'
import { PdErrorRow } from '@/components/PdErrorRow'
import { PdLogs } from '@/components/PdLogs'
import { SUPPORTED_LOCALES, type Locale } from '@/i18n'
import { engineInfo, type EngineInfo } from '@/ipc'
import { useEnvStore, useSettingsStore } from '@/stores'

const ENGINES = ['pip', 'uv'] as const

function EngineVersion({ info }: { info: EngineInfo | undefined }) {
  const { t } = useTranslation()
  if (info === undefined) return null
  return (
    <span data-engine-version={info.id} className="font-mono text-data text-text-dim">
      {info.available ? (info.version ?? '') : t('settings.engineMissing')}
    </span>
  )
}

export function PdSettings() {
  const { t } = useTranslation()
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

  // Turning the override on is asked about first, with Cancel focused (SECURITY §3, UI-SPEC §7).
  // It was one click on a checkbox — and, until 1.4.0, a click nothing read, so nobody noticed.
  const [confirmingOverride, setConfirmingOverride] = useState(false)

  useEffect(() => {
    void load()
  }, [load])

  // ARCHITECTURE §7's `engine_info`: a version beside each radio, for the selected environment —
  // pip's comes from `<python> -m pip --version`, so there is no answer without one. The wrapper
  // shipped with no caller, so nothing said uv was missing until a plan failed for want of it.
  // Keyed by interpreter, so a switch never shows one environment's versions under another.
  const env = useEnvStore((s) => s.rows.find((r) => r.interpreter === s.selected)?.env)
  const [engines, setEngines] = useState<{ for: string; info: EngineInfo[] } | null>(null)
  useEffect(() => {
    if (env === undefined) return undefined
    let alive = true
    void engineInfo(env).then(
      (info) => {
        if (alive) setEngines({ for: env.interpreter, info })
      },
      () => undefined,
    )
    return () => {
      alive = false
    }
  }, [env])
  const infoFor = (id: string) =>
    engines !== null && engines.for === env?.interpreter
      ? engines.info.find((i) => i.id === id)
      : undefined

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
              {/* So is a version; only "not found" is copy. */}
              <EngineVersion info={infoFor(id)} />
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
                  // I18N §2: applied live — and stored, so it is still the language tomorrow.
                  void setLocale(code)
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
              // Off is immediate: turning protection back on never needs a second thought.
              if (e.target.checked) {
                setConfirmingOverride(true)
              } else {
                void save({ allowExternallyManaged: false })
              }
            }}
            className="mt-1 accent-[var(--color-danger)]"
          />
          <span className="text-data text-warn">{t('settings.overrideDetail')}</span>
        </label>
      </fieldset>

      {confirmingOverride ? (
        <PdDialog
          label={t('settings.overrideConfirmTitle')}
          title={t('settings.overrideConfirmTitle')}
          cancelLabel={t('actions.cancel')}
          onCancel={() => {
            setConfirmingOverride(false)
          }}
          actions={
            <button
              type="button"
              onClick={() => {
                setConfirmingOverride(false)
                void save({ allowExternallyManaged: true })
              }}
              data-action="allow-override"
              className="rounded-pd border border-danger px-3 py-1 text-data text-danger"
            >
              {t('settings.overrideConfirm')}
            </button>
          }
        >
          <p className="text-data">{t('settings.overrideConfirmBody')}</p>
        </PdDialog>
      ) : null}

      <PdCacheUsage />
      <PdLogs />
    </section>
  )
}
