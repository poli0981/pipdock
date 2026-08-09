import { useEffect } from 'react'
import { useTranslation } from 'react-i18next'

import { NAV_KEYS } from '@/components/nav'
import { PdLegalGate } from '@/components/PdLegalGate'
import { PdOfflineBanner } from '@/components/PdOfflineBanner'
import { PdSidebar } from '@/components/PdSidebar'
import { PdStatusLine } from '@/components/PdStatusLine'
import { PdUninstallDialog } from '@/components/PdUninstallDialog'
import type { NavKey } from '@/components/nav'
import { PdEnvironments } from '@/screens/PdEnvironments'
import { PdPackages } from '@/screens/PdPackages'
import { PdPins } from '@/screens/PdPins'
import { PdPlanPanel } from '@/screens/PdPlanPanel'
import { PdSearch } from '@/screens/PdSearch'
import { PdSettings } from '@/screens/PdSettings'
import { PANEL_PHASES } from '@/stores/plan'
import { useEnvStore, useLegalStore, usePlanStore, useUiStore } from '@/stores'

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
  pins: <PdPins />,
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
  const loadPackages = useEnvStore((s) => s.loadPackages)
  const loadOutdated = useEnvStore((s) => s.loadOutdated)
  const clearSelection = useEnvStore((s) => s.clearSelection)
  const loadSnapshots = useEnvStore((s) => s.loadSnapshots)
  // A plan belongs to the app, not to a screen: there is one at a time (PD-RES-003) and it can be
  // started from Updates *or* from the Search dock bay. Owning it here is what stops an install
  // resolving into nowhere because the tab that renders the preview is not the tab you are on.
  const planPhase = usePlanStore((s) => s.phase)
  const guard = usePlanStore((s) => s.guard)
  const guardBusy = usePlanStore((s) => s.guardBusy)
  const widen = usePlanStore((s) => s.widen)
  const confirmUninstall = usePlanStore((s) => s.confirmUninstall)
  const resetPlan = usePlanStore((s) => s.reset)
  const guardOpen = planPhase === 'guard' && guard !== null

  useEffect(() => {
    void checkConsent()
  }, [checkConsent])

  // UI-SPEC §8: Ctrl+1..8 select tabs, positionally over NAV_KEYS.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // A native <dialog> makes the rest of the document inert to pointers and to focus, but a
      // window-level keydown listener is neither — so without this, Ctrl+3 changes the tab
      // underneath an open guard dialog and the user answers it about a screen they left.
      if (guardOpen) return
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
  }, [setNav, guardOpen])

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
          {PANEL_PHASES.has(planPhase) ? (
            <PdPlanPanel
              onFinished={() => {
                // The environment changed under whatever screen is behind this, so re-read it
                // rather than showing the versions from before the run.
                clearSelection()
                void loadPackages()
                void loadOutdated()
                // A rollback writes its own pre-rollback snapshot, so the timeline behind this
                // panel is stale the moment the run finishes. `force`, because `snapshotsFor`
                // still matches and would suppress the refetch.
                void loadSnapshots(true)
              }}
            />
          ) : (SCREENS[nav] ?? (
            <p className="h-full overflow-auto p-6 font-mono text-text-dim">
              {`▸ ${t(`nav.${nav}`)}`}
            </p>
          ))}
        </main>
      </div>

      <PdStatusLine />

      {/* Outside <main>, because the dialog opens *over* the table the user selected from — they
          need to still see what they picked while deciding. Mounted here rather than in the row
          that opened it: `PdPackageTable` renders a ~25-row window, so scrolling would unmount
          the dialog's own parent underneath it. */}
      {guardOpen ? (
        <PdUninstallDialog
          report={guard}
          busy={guardBusy}
          onCancel={resetPlan}
          onWiden={() => {
            void widen()
          }}
          onConfirm={(force) => {
            void confirmUninstall(force)
          }}
        />
      ) : null}
    </div>
  )
}
