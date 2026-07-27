import { useTranslation } from 'react-i18next'

import { PdSidebar } from '@/components/PdSidebar'
import { PdStatusLine } from '@/components/PdStatusLine'

/**
 * The app shell — UI-SPEC §3.
 *
 * Top bar, sidebar, content area and a pinned terminal-style status line. The console drawer
 * slides up over the status line during execution; screens fill the content area in M2.
 */
export function App() {
  const { t } = useTranslation()

  return (
    <div className="flex h-full flex-col bg-bg text-text">
      <header className="flex items-center justify-between border-b border-border px-4 py-2">
        <span className="font-mono text-accent">{t('app.name')}</span>
        <span className="font-mono text-[13px] text-text-dim">{t('status.noEnvironment')}</span>
      </header>

      <div className="flex min-h-0 flex-1">
        <PdSidebar />
        <main className="min-w-0 flex-1 overflow-auto p-6">
          <h1 className="text-accent">{t('phase0.heading')}</h1>
          <p className="mt-2 max-w-prose text-text-dim">{t('phase0.body')}</p>
          <p className="mt-4 font-mono text-[13px] text-accent-dim">{t('phase0.docs')}</p>
        </main>
      </div>

      <PdStatusLine />
    </div>
  )
}
