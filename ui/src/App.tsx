import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { isBrowserShortcut } from '@/components/browserKeys'
import { NAV_KEYS } from '@/components/nav'
import { PdLegalGate } from '@/components/PdLegalGate'
import { PdOfflineBanner } from '@/components/PdOfflineBanner'
import { PdPalette } from '@/components/PdPalette'
import { PdSidebar } from '@/components/PdSidebar'
import { PdStatusLine } from '@/components/PdStatusLine'
import { PdUninstallDialog } from '@/components/PdUninstallDialog'
import type { NavKey } from '@/components/nav'
import { PdAbout } from '@/screens/PdAbout'
import { PdEnvironments } from '@/screens/PdEnvironments'
import { PdHealth } from '@/screens/PdHealth'
import { PdPackages } from '@/screens/PdPackages'
import { PdPins } from '@/screens/PdPins'
import { PdPlanPanel } from '@/screens/PdPlanPanel'
import { PdSearch } from '@/screens/PdSearch'
import { PdSecurity } from '@/screens/PdSecurity'
import { PdSettings } from '@/screens/PdSettings'
import { PANEL_PHASES } from '@/stores/plan'
import {
  useDepsStore,
  useEnvStore,
  useLegalStore,
  usePlanStore,
  useSettingsStore,
  useUiStore,
} from '@/stores'

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
  health: <PdHealth />,
  security: <PdSecurity />,
  settings: <PdSettings />,
  about: <PdAbout />,
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
  const review = useLegalStore((s) => s.review)
  const closeReview = useLegalStore((s) => s.closeReview)
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
  const resetDeps = useDepsStore((s) => s.reset)
  const guardOpen = planPhase === 'guard' && guard !== null
  const mainRef = useRef<HTMLElement>(null)
  // Shell state, like `nav` — but local, because nothing outside this component reads it.
  const [paletteOpen, setPaletteOpen] = useState(false)

  useEffect(() => {
    void checkConsent()
  }, [checkConsent])

  // Settings are the app's, not the Settings screen's: the status line names the engine, the
  // PEP 668 chip reads the override, and the language is the user's. Loaded only when Settings
  // mounted, so all three showed the defaults until someone opened Settings (U13) — and loaded
  // before the legal gate rather than after it, so the gate is in the language they chose (U3).
  const loadSettings = useSettingsStore((s) => s.load)
  useEffect(() => {
    void loadSettings()
  }, [loadSettings])

  // U20: a release build has no reload, print or find to offer — see `browserKeys`. Capture phase
  // and above the gate's early return, so no screen and no modal can let one through.
  useEffect(() => {
    if (!import.meta.env.PROD) return undefined
    const swallow = (e: KeyboardEvent) => {
      if (isBrowserShortcut(e)) e.preventDefault()
    }
    window.addEventListener('keydown', swallow, { capture: true })
    return () => {
      window.removeEventListener('keydown', swallow, { capture: true })
    }
  }, [])

  // UI-SPEC §8's "full keyboard traversal": after Ctrl+3 the focus was still on whatever the user
  // last clicked, so the next Tab resumed in the old screen and a screen reader announced nothing.
  // Moving it to the new screen's heading is the smallest thing that makes the shortcut complete.
  // `key={nav}` on <main> forces a remount, so this fires on every change rather than only the
  // first — and the heading takes `tabIndex={-1}` from the CSS-free `[tabindex]` below.
  //
  // A screen that names its own first stop with `data-autofocus` gets that instead. Search's field
  // is UI-SPEC §5's first click, and PdSearch focused it — then this effect, which runs after its
  // children's, moved focus to the heading on every arrival (U12).
  useEffect(() => {
    const main = mainRef.current
    const own = main?.querySelector<HTMLElement>('[data-autofocus]')
    if (own !== null && own !== undefined) {
      own.focus()
      return
    }
    const heading = main?.querySelector('h1')
    if (heading === null || heading === undefined) return
    heading.setAttribute('tabindex', '-1')
    heading.focus()
  }, [nav])

  // UI-SPEC §8: Ctrl+1..9 select tabs, positionally over NAV_KEYS.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Nothing is navigable before the documents are accepted. This effect is registered *above*
      // the `accepted` early return, so without the check the shortcuts stayed live behind the
      // modal and the user could change tabs behind a document they were being asked to agree to.
      if (accepted !== true || review) return
      // A native <dialog> makes the rest of the document inert to pointers and to focus, but a
      // window-level keydown listener is neither — so without this, Ctrl+3 changes the tab
      // underneath an open guard dialog and the user answers it about a screen they left.
      //
      // `PANEL_PHASES` is the same rule for the panel. UI-SPEC §8 has claimed since S3 that the
      // shortcuts already refused there; they did not. `guardOpen` and `PANEL_PHASES` are
      // *disjoint* — `plan.ts` excludes `guard` from the set on purpose — so guarding on one
      // guarded neither. The sidebar was disabled and the panel kept rendering, so nothing looked
      // wrong until the plan finished and dropped the user on a tab they never chose.
      if (guardOpen || PANEL_PHASES.has(planPhase)) return

      // UI-SPEC §1's Ctrl+K palette. Before the Ctrl+digit branch below, which parses `e.key` as a
      // number — `'k'` becomes NaN and `NAV_KEYS[NaN]` is undefined, so this was a silent no-op
      // rather than a collision. `toLowerCase`, because Shift+Ctrl+K reports `'K'`.
      //
      // It inherits both refusals above by construction: this line is unreachable while the legal
      // gate is up, while the guard dialog is open, or while a plan owns the content area — which
      // is the point. A palette that can navigate during an execution is the defect S3 shipped,
      // reached through a different door.
      if (e.ctrlKey && !e.altKey && !e.metaKey && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        setPaletteOpen((open) => !open)
        return
      }
      // UI-SPEC §8: `/` focuses search. Guarded against text fields, or it would be impossible to
      // type a slash into a version specifier.
      const target = e.target as HTMLElement | null
      const typing =
        target?.tagName === 'INPUT' || target?.tagName === 'TEXTAREA' || target?.isContentEditable
      if (e.key === '/' && !e.ctrlKey && !e.altKey && !e.metaKey && typing !== true) {
        e.preventDefault()
        // Already on Search, `nav` does not change and the focus effect above does not run, so
        // `/` did nothing at all from the very screen it names.
        if (nav === 'search') {
          mainRef.current?.querySelector<HTMLElement>('[data-autofocus]')?.focus()
        } else {
          setNav('search')
        }
        return
      }

      if (!e.ctrlKey || e.altKey || e.metaKey) return
      // By physical key (U49): `e.key` is what the layout types, and on AZERTY Ctrl+1 reports `&`,
      // so the tab shortcuts did nothing for every French keyboard.
      const digit = /^(?:Digit|Numpad)([1-9])$/.exec(e.code)?.[1]
      const key = digit === undefined ? undefined : NAV_KEYS[Number(digit) - 1]
      if (key !== undefined) {
        e.preventDefault()
        setNav(key)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('keydown', onKey)
    }
  }, [nav, setNav, guardOpen, planPhase, accepted, review])

  // Nothing is usable before the documents are accepted, so the gate replaces the shell rather
  // than overlaying a working app (UI-SPEC §4).
  if (accepted === false) return <PdLegalGate />
  // `null` means the check has not resolved yet. Rendering the shell first would flash it at
  // someone who has never agreed to anything.
  if (accepted === null) return <div className="h-full bg-bg" />
  // Opened from About, after acceptance. Same documents, no checkbox, no Decline — see the store's
  // note on why this is a flag of its own rather than `accepted = false`.
  if (review) return <PdLegalGate review onClose={closeReview} />

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
        {/* `mx-auto max-w-content`: the window maximizes, the column does not (UI-SPEC §5). */}
        <main
          key={nav}
          ref={mainRef}
          className="mx-auto flex min-w-0 max-w-content flex-1 flex-col overflow-hidden"
        >
          {PANEL_PHASES.has(planPhase) ? (
            <PdPlanPanel
              onFinished={(changed) => {
                // Nothing ran: the screen behind the panel is still true, and the selection is
                // still what the user picked.
                if (!changed) return
                // The environment changed under whatever screen is behind this, so re-read it
                // rather than showing the versions from before the run.
                clearSelection()
                void loadPackages()
                void loadOutdated()
                // A rollback writes its own pre-rollback snapshot, so the timeline behind this
                // panel is stale the moment the run finishes. `force`, because `snapshotsFor`
                // still matches and would suppress the refetch.
                void loadSnapshots(true)
                // The graph is stale too, and this is the one slice whose freshness guard cannot
                // see it: `freshGraph` keys on `envHash`, which a mutation does not change. Health
                // and Security guard and never reset because nothing they read is altered by an
                // install; edges are exactly what an install alters. Cleared rather than
                // refetched, so the probe is paid again only if the view is opened again.
                resetDeps()
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

      {/* Outside <main> for `PdUninstallDialog`'s reason: it opens over whatever screen is
          showing, and mounting it inside one would tie it to that screen's lifetime. */}
      {paletteOpen ? (
        <PdPalette
          onClose={() => {
            setPaletteOpen(false)
          }}
        />
      ) : null}

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
