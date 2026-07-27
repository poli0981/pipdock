import { useTranslation } from 'react-i18next'

import { useSettingsStore } from '@/stores'

/**
 * The status line — UI-SPEC §3.
 *
 * Always shows env · python · engine · state in monospace, plus the log-drawer toggle. The
 * engine badge carries the live-activity pulse during execution (UI-SPEC §2).
 */
export function PdStatusLine() {
  const { t } = useTranslation()
  const engine = useSettingsStore((s) => s.engine)

  return (
    <footer className="flex items-center gap-3 border-t border-border bg-surface px-4 py-1.5 font-mono text-[13px] text-text-dim">
      <span aria-hidden="true">{'▸'}</span>
      <span>{t('status.noEnvironment')}</span>
      <span aria-hidden="true">{'·'}</span>
      {/* Engine ids are never translated (docs/I18N.md §2). */}
      <span className="text-accent">{engine}</span>
      <span aria-hidden="true">{'·'}</span>
      <span>{t('status.idle')}</span>
      <span className="ml-auto">{t('status.log')}</span>
    </footer>
  )
}
