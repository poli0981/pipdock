/**
 * Pins — UI-SPEC §4: "Pin list with reason field".
 *
 * The whole specification is that sentence, so the shape comes from what a pin has to answer six
 * months later: *which* package, *why*, and how to stop. There is no `PdPins*` in §6's component
 * inventory, so it is built from the primitives; the 🔒 chip is `PdPinChip`, shared with the table
 * rather than re-derived, or the two would eventually disagree about which mode is which.
 *
 * **Exclude only.** The screen shows a `Hold` pin faithfully if one exists — the CLI could write
 * one in principle — but does not offer to create one, because `pins::hold_requirements` is dead
 * code in the core and `engine::plan_requirements` restates each package at its *installed*
 * version. A hold at any other version is a promise nothing keeps, and offering it here would be
 * the interface lying about what the engine does.
 *
 * The reason commits on blur rather than on every keystroke: it is a `pin_add` upsert per commit,
 * and one round trip per character typed is a lot of SQLite for a text field.
 *
 * **The suggestion section is §4's, and it lands here rather than anywhere louder.** It costs one
 * `probe.py` run, so it is paid by someone who opened this tab — a sidebar badge would charge
 * every user, including those who never pin anything. It is also silent on failure: this screen's
 * job is listing pins, and an error row about an *advisory* would be the loudest thing on it.
 */

import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { PdEmptyState } from '@/components/PdEmptyState'
import { PdErrorRow } from '@/components/PdErrorRow'
import { PdPinChip } from '@/components/PdPinChip'
import { useEnvPackages } from '@/screens/useEnvPackages'
import { useEnvStore } from '@/stores'

export function PdPins() {
  const { t } = useTranslation()
  const selected = useEnvStore((s) => s.selected)
  const pins = useEnvStore((s) => s.pins)
  const listError = useEnvStore((s) => s.listError)
  const togglePin = useEnvStore((s) => s.togglePin)
  const updatePin = useEnvStore((s) => s.updatePin)
  const suggestions = useEnvStore((s) => s.suggestions)
  const loadSuggestions = useEnvStore((s) => s.loadSuggestions)
  const acceptSuggestion = useEnvStore((s) => s.acceptSuggestion)

  useEnvPackages()

  // Its own effect rather than a third fetch inside `useEnvPackages`: that hook's two calls share
  // one `loadedFor` flag, and folding a probe-costing third into it would tie its staleness to
  // theirs. The store keys this on `suggestionsFor` and refuses a concurrent second call.
  useEffect(() => {
    if (selected !== null) void loadSuggestions()
  }, [selected, loadSuggestions])

  return (
    <section aria-labelledby="pins-title" className="h-full overflow-auto p-6">
      <h1 id="pins-title" className="text-accent">
        {t('nav.pins')}
      </h1>

      {listError !== null ? (
        <div className="mt-4">
          <PdErrorRow error={listError} />
        </div>
      ) : null}

      {/* Above the list, because it is a prompt to act and the list below is a record of having
          acted. Absent entirely when there is nothing to suggest — an empty "Worth pinning"
          heading is a question nobody asked. */}
      {selected !== null && suggestions.length > 0 ? (
        <section aria-labelledby="pins-suggest" className="mt-4">
          <h2 id="pins-suggest" className="text-text-dim">
            {t('pins.suggestTitle')}
          </h2>
          <p className="mt-1 max-w-2xl text-data text-text-dim">{t('pins.suggestIntro')}</p>
          <ul className="mt-2 space-y-1">
            {suggestions.map((s) => (
              <li
                key={s.pkg}
                data-suggestion={s.pkg}
                className="flex items-center gap-3 rounded-pd border border-border bg-surface px-3 py-1.5"
              >
                {/* UI-SPEC §4 fixes this sentence: "urllib3 — 12 packages depend on it." The name
                    is data and is interpolated, never translated (I18N §2). */}
                <span className="min-w-0 flex-1 text-data">
                  {t('pins.suggest', { pkg: s.pkg, count: s.dependents })}
                </span>
                <button
                  type="button"
                  aria-label={t('pins.suggestAcceptFor', { pkg: s.pkg })}
                  onClick={() => {
                    // PRD P1-2: "suggest pin with reason". The count is what justified the
                    // suggestion, so it is what the pin records — and the field below is
                    // editable, so the user can replace it with their own.
                    void acceptSuggestion(
                      s.pkg,
                      t('pins.suggestReason', { count: s.dependents }),
                    )
                  }}
                  data-action="accept-suggestion"
                  className="shrink-0 rounded-pd border border-border px-2 py-0.5 text-data text-text-dim hover:bg-surface-2"
                >
                  {t('pins.suggestAccept')}
                </button>
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      {selected === null ? (
        // Pins are stored per `envHash`, so with no environment there is not an empty list — there
        // is no list. Saying "no pins" here would read as "this environment has none".
        <PdEmptyState
          message={t('packages.noEnvironment')}
          hint={t('packages.noEnvironmentHint')}
        />
      ) : pins.length === 0 ? (
        <PdEmptyState message={t('pins.empty')} hint={t('pins.emptyHint')} />
      ) : (
        <ul className="mt-4 space-y-2">
          {pins.map((pin) => (
            <li
              key={pin.pkg}
              data-pin={pin.pkg}
              className="flex items-center gap-3 rounded-pd border border-border bg-surface px-3 py-2"
            >
              <code className="w-56 shrink-0 truncate font-mono text-data">{pin.pkg}</code>
              <PdPinChip mode={pin.mode} />
              <ReasonField
                // Keyed on the committed reason, which is React's own way to reset state when a
                // prop changes: a write elsewhere — unpinning another row re-reads the whole
                // list — must not strand this box showing what it held before. The key is stable
                // while typing, because typing only moves local draft state.
                key={`${pin.pkg}:${pin.reason ?? ''}`}
                pkg={pin.pkg}
                reason={pin.reason ?? ''}
                onCommit={(next) => {
                  void updatePin(pin.pkg, next)
                }}
              />
              <button
                type="button"
                onClick={() => {
                  void togglePin(pin.pkg)
                }}
                data-action="unpin"
                className="shrink-0 rounded-pd border border-border px-2 py-0.5 text-data text-text-dim"
              >
                {t('packages.actions.unpin')}
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

/** The reason box for one pin: local while typing, committed on blur. */
function ReasonField({
  pkg,
  reason,
  onCommit,
}: {
  pkg: string
  reason: string
  onCommit: (reason: string | null) => void
}) {
  const { t } = useTranslation()
  const [draft, setDraft] = useState(reason)

  return (
    <input
      type="text"
      value={draft}
      aria-label={t('pins.reasonFor', { pkg })}
      placeholder={t('pins.reasonPlaceholder')}
      onChange={(e) => {
        setDraft(e.target.value)
      }}
      onBlur={() => {
        const next = draft.trim()
        if (next === reason) return
        // `null`, never `''`: `exactOptionalPropertyTypes` is on and `reason` is an optional
        // field, so an empty string would round-trip through SQLite as a reason that is there and
        // says nothing.
        onCommit(next === '' ? null : next)
      }}
      className="min-w-0 flex-1 rounded-pd border border-border bg-bg px-2 py-0.5 text-data"
    />
  )
}
