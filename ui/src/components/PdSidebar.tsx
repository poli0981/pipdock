import { useTranslation } from 'react-i18next'

import { NAV_KEYS } from '@/components/nav'

/**
 * The sidebar — UI-SPEC §3 and §8.
 *
 * Icon + label, collapsible to icons. `Ctrl+1..8` selects a tab; the keyboard map lands with the
 * screens in M2, so the entries render but are inert here.
 */
export function PdSidebar() {
  const { t } = useTranslation()

  return (
    <nav aria-label={t('nav.environments')} className="w-48 shrink-0 border-r border-border p-2">
      <ul className="space-y-0.5">
        {NAV_KEYS.map((key) => (
          <li key={key}>
            <span className="block rounded-[6px] px-3 py-1.5 text-text-dim">{t(`nav.${key}`)}</span>
          </li>
        ))}
      </ul>
    </nav>
  )
}
