import { useEffect } from 'react'
import { useTranslation } from 'react-i18next'

import { NAV_KEYS } from '@/components/nav'
import { PdLegalGate } from '@/components/PdLegalGate'
import { PdOfflineBanner } from '@/components/PdOfflineBanner'
import { PdSidebar } from '@/components/PdSidebar'
import { PdStatusLine } from '@/components/PdStatusLine'
import type { NavKey } from '@/components/nav'
import { PdEnvironments } from '@/screens/PdEnvironments'
import { PdPackages } from '@/screens/PdPackages'
import { PdSearch } from '@/screens/PdSearch'
import { PdSettings } from '@/screens/PdSettings'
import { useEnvStore, useLegalStore, useUiStore } from '@/stores'

/**
 * Which screen each tab shows. A lookup rather than a ternary chain: the chain's last branch was a
 * hand-maintained negation of every key above it, so adding a screen meant editing two places and
 * forgetting the second showed both at once. Tabs with no entry fall through to the placeholder.
 */
const SCREENS: Partial<Record<NavKey, React.ReactNode>> = {
  environments: <PdEnvironments />,
  installed: <PdPackages mode="installed" />,
  updates: <PdPackages mode="updates" />,
  search: <PdSearch />,
  settings: <PdSettings />,
}

/**
 * The app shell — UI-SPEC §3.
 *
 * Top bar, sidebar, content area and a pinned terminal-style status line. The console drawer
 * slides up over the status line during execution; the remaining screens land with their slices.
 */
export function App() {
  const { t } = useTranslation()
  const nav = useUiStore((s) => s.nav)
  const setNav = useUiStore((s) => s.setNav)
  const accepted = useLegalStore((s) => s.accepted)
  const checkConsent = useLegalStore((s) => s.check)
  const selected = useEnvStore((s) => s.selected)

  useEffect(() => {
    void checkConsent()
  }, [checkConsent])

  // UI-SPEC §8: Ctrl+1..8 select tabs, positionally over NAV_KEYS.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // UI-SPEC §8: `/` focuses search. Guarded against text fields, or it would be impossible to
      // type a slash into a version specifier.
      const target = e.target as HTMLElement | null
      const typing =
        target?.tagName === 'INPUT' || target?.tagName === 'TEXTAREA' || target?.isContentEditable
      if (e.key === '/' && !e.ctrlKey && !e.altKey && !e.metaKey && typing !== true) {
        e.preventDefault()
        setNav('search')
        return
      }

      if (!e.ctrlKey || e.altKey || e.metaKey) return
      const index = Number.parseInt(e.key, 10) - 1
      const key = NAV_KEYS[index]
      if (key !== undefined) {
        e.preventDefault()
        setNav(key)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('keydown', onKey)
    }
  }, [setNav])

  // Nothing is usable before the documents are accepted, so the gate replaces the shell rather
  // than overlaying a working app (UI-SPEC §4).
  if (accepted === false) return <PdLegalGate />
  // `null` means the check has not resolved yet. Rendering the shell first would flash it at
  // someone who has never agreed to anything.
  if (accepted === null) return <div className="h-full bg-bg" />

  return (
    <div className="flex h-full flex-col bg-bg text-text">
      <header className="flex items-center justify-between border-b border-border px-4 py-2">
        <span className="font-mono text-accent">{t('app.name')}</span>
        <span className="flex items-center gap-2 font-mono text-data text-text-dim">
          <PdOfflineBanner />
          {selected ?? t('status.noEnvironment')}
        </span>
      </header>

      <div className="flex min-h-0 flex-1">
        <PdSidebar />
        {/* Each screen owns its own scroll container. A virtualizer needs a scroll element it
            can observe, and nesting one inside an already-scrolling <main> gives two scrollbars
            and an outer one whose content is already the virtualizer's full height. */}
        <main className="flex min-w-0 flex-1 flex-col overflow-hidden">
          {SCREENS[nav] ?? (
            <p className="h-full overflow-auto p-6 font-mono text-text-dim">
              {`▸ ${t(`nav.${nav}`)}`}
            </p>
          )}
        </main>
      </div>

      <PdStatusLine />
    </div>
  )
}
