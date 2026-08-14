import { useTranslation } from 'react-i18next'

import { NAV_KEYS, type NavKey } from '@/components/nav'
import { PANEL_PHASES, useEnvStore, usePlanStore, useUiStore } from '@/stores'

/** Tabs whose screens land in a later milestone. Kept in place, not hidden. */
const NOT_YET: readonly NavKey[] = ['security']

/**
 * The sidebar — UI-SPEC §3 and §8.
 *
 * `Ctrl+1..9` maps to `NAV_KEYS` positionally, which is why that order is load-bearing. Tabs
 * without a screen yet stay in place and stay focusable rather than being hidden: hiding them
 * would renumber every shortcut after them, and the user would relearn the map twice.
 *
 * **Every tab is disabled while a plan owns the content area.** `PdPlanPanel` replaces `<main>`
 * for the whole of a preview, an execution and its summary; navigating away left the plan parked
 * in Rust with nothing on screen driving it, and the user's next Update answered `PD-RES-003`
 * about a plan they could no longer see. `App.tsx` refuses `Ctrl+1..9` while the guard
 * dialog is open, for the same reason — this is that rule applied to the panel and to the mouse.
 */
export function PdSidebar() {
  const { t } = useTranslation()
  const nav = useUiStore((s) => s.nav)
  const setNav = useUiStore((s) => s.setNav)
  // UI-SPEC §3's layout sketch shows `Updates (7)`. A primitive, so this selector is stable.
  const updatesCount = useEnvStore((s) => s.updatesCount)
  // A boolean rather than the phase, so an unrelated phase change does not re-render the sidebar.
  const planBusy = usePlanStore((s) => PANEL_PHASES.has(s.phase))

  return (
    <nav aria-label={t('app.name')} className="w-48 shrink-0 border-r border-border p-2">
      <ul className="space-y-0.5">
        {NAV_KEYS.map((key, index) => {
          const active = nav === key
          return (
            <li key={key}>
              <button
                type="button"
                aria-current={active ? 'page' : undefined}
                disabled={planBusy}
                onClick={() => {
                  setNav(key)
                }}
                className={`flex w-full items-center justify-between rounded-pd px-3 py-1.5 text-left ${
                  active ? 'bg-surface-2 text-accent' : 'text-text-dim hover:bg-surface-2'
                } ${NOT_YET.includes(key) ? 'opacity-60' : ''} ${
                  planBusy ? 'cursor-not-allowed opacity-40' : ''
                }`}
              >
                <span>
                  {t(`nav.${key}`)}
                  {key === 'updates' && updatesCount > 0 ? (
                    <span className="ml-1 font-mono text-data text-warn">
                      {`(${String(updatesCount)})`}
                    </span>
                  ) : null}
                </span>
                <span aria-hidden="true" className="font-mono text-data opacity-50">
                  {`^${index + 1}`}
                </span>
              </button>
            </li>
          )
        })}
      </ul>
    </nav>
  )
}
