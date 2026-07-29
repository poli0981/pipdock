import { useTranslation } from 'react-i18next'

import { NAV_KEYS, type NavKey } from '@/components/nav'
import { useUiStore } from '@/stores'

/** Tabs whose screens land in a later milestone. Kept in place, not hidden. */
const NOT_YET: readonly NavKey[] = ['installed', 'updates', 'search', 'pins', 'health', 'security']

/**
 * The sidebar — UI-SPEC §3 and §8.
 *
 * `Ctrl+1..8` maps to `NAV_KEYS` positionally, which is why that order is load-bearing. Tabs
 * without a screen yet stay in place and stay focusable rather than being hidden: hiding them
 * would renumber every shortcut after them, and the user would relearn the map twice.
 */
export function PdSidebar() {
  const { t } = useTranslation()
  const nav = useUiStore((s) => s.nav)
  const setNav = useUiStore((s) => s.setNav)

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
                onClick={() => {
                  setNav(key)
                }}
                className={`flex w-full items-center justify-between rounded-pd px-3 py-1.5 text-left ${
                  active ? 'bg-surface-2 text-accent' : 'text-text-dim hover:bg-surface-2'
                } ${NOT_YET.includes(key) ? 'opacity-60' : ''}`}
              >
                <span>{t(`nav.${key}`)}</span>
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
