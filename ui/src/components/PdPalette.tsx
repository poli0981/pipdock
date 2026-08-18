/**
 * The command palette — `Ctrl+K`, PRD P1-5 and UI-SPEC §1's "signature move".
 *
 * Frontend only: every action it dispatches already exists as a store action, so there is no new
 * IPC command, no Rust and no capability change.
 *
 * **Not built on `PdDialog`**, which hardcodes a heading and a Cancel-first button row for
 * destructive confirms — the opposite of what a filter list wants. What is lifted from it is the
 * part worth reusing: a native `<dialog>` with `showModal()`, which brings the top layer, the
 * inert backdrop, real focus containment and `Esc` for free, plus the `if (!el.open)` guard that
 * exists because StrictMode double-invokes the mount effect.
 *
 * **It dismisses on backdrop click**, unlike `PdDialog`. That component argues the opposite and is
 * right to: a mis-click beside a dialog about to delete things must not read as consent. Nothing
 * here is destructive on its own — every action either navigates or opens something that has its
 * own confirm — so the convention wins.
 *
 * The list is a `listbox` with `aria-activedescendant`, not a set of tab stops: the input keeps
 * focus while `↑`/`↓` move the selection, which is what lets someone keep typing to narrow.
 * `aria-selected` is what makes the highlight survive Windows high-contrast mode, where the
 * `bg-surface-2` tint is erased and `styles.css`'s forced-colors block outlines the selected row
 * instead.
 */

import { useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { NAV_KEYS } from '@/components/nav'
import { rank } from '@/components/match'
import { useEnvStore, useHealthStore, useIndexStore, useLegalStore, useUiStore } from '@/stores'

/** One thing the palette can do. */
interface Action {
  /** Stable, and the React key. */
  id: string
  /** Already localized. */
  label: string
  /** Shown dimmed on the right — which screen or area it belongs to. */
  group: string
  run: () => void
}

export function PdPalette({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation()
  const ref = useRef<HTMLDialogElement>(null)
  const [query, setQuery] = useState('')
  const [at, setAt] = useState(0)

  const setNav = useUiStore((s) => s.setNav)
  const scan = useEnvStore((s) => s.scan)
  const loadPackages = useEnvStore((s) => s.loadPackages)
  const loadOutdated = useEnvStore((s) => s.loadOutdated)
  const takeSnapshot = useEnvStore((s) => s.takeSnapshot)
  const clearSelection = useEnvStore((s) => s.clearSelection)
  const refreshIndex = useIndexStore((s) => s.refreshIndex)
  const clearQueue = useIndexStore((s) => s.clearQueue)
  const setTab = useHealthStore((s) => s.setTab)
  const openReview = useLegalStore((s) => s.openReview)

  const actions: Action[] = useMemo(
    () => [
      // The nine tabs, every one of which now has a screen: `security` was the last placeholder
      // and P1-1 filled it.
      ...NAV_KEYS.map((key) => ({
        id: `nav:${key}`,
        label: t(`nav.${key}`),
        group: t('palette.groups.go'),
        run: () => {
          setNav(key)
        },
      })),
      {
        id: 'env:rescan',
        label: t('actions.rescan'),
        group: t('nav.environments'),
        run: () => {
          setNav('environments')
          void scan()
        },
      },
      {
        id: 'pkg:refresh',
        label: t('palette.refreshPackages'),
        group: t('nav.installed'),
        run: () => {
          void loadPackages()
          void loadOutdated()
        },
      },
      {
        id: 'pkg:clear-selection',
        label: t('palette.clearSelection'),
        group: t('nav.installed'),
        run: clearSelection,
      },
      {
        id: 'snapshot:take',
        label: t('snapshots.take'),
        group: t('snapshots.title'),
        run: () => {
          void takeSnapshot()
        },
      },
      {
        id: 'index:refresh',
        label: t('search.refreshIndex'),
        group: t('nav.search'),
        run: () => {
          void refreshIndex()
        },
      },
      {
        id: 'index:clear-queue',
        label: t('palette.clearQueue'),
        group: t('nav.search'),
        run: clearQueue,
      },
      ...(['deptry', 'vulture', 'ruff'] as const).map((tool) => ({
        id: `health:${tool}`,
        // The tool's own name, which is data and never translated (I18N §2).
        label: `${t('nav.health')}: ${tool}`,
        group: t('nav.health'),
        run: () => {
          setNav('health')
          setTab(tool)
        },
      })),
      {
        id: 'legal:review',
        label: t('about.reopen'),
        group: t('nav.about'),
        run: openReview,
      },
    ],
    [
      t,
      setNav,
      scan,
      loadPackages,
      loadOutdated,
      clearSelection,
      takeSnapshot,
      refreshIndex,
      clearQueue,
      setTab,
      openReview,
    ],
  )

  const shown = useMemo(() => rank(actions, query, (a) => [a.label, a.group]), [actions, query])
  // Clamped rather than reset: narrowing the list must not silently move the selection to
  // something the user was not looking at.
  const active = Math.min(at, Math.max(0, shown.length - 1))

  useEffect(() => {
    const el = ref.current
    // `showModal` throws when the dialog is already open, which React's double-invoked effects in
    // development make a real case rather than a theoretical one.
    if (el !== null && !el.open) el.showModal()
    return () => {
      if (el !== null && el.open) el.close()
    }
  }, [])

  const dispatch = (action: Action | undefined) => {
    if (action === undefined) return
    // Closed *before* running, so an action that changes `nav` does not fight the dialog for
    // focus — `App.tsx` moves focus to the new screen's `<h1>` on every nav change.
    onClose()
    action.run()
  }

  return (
    <dialog
      ref={ref}
      aria-label={t('palette.title')}
      onCancel={(e) => {
        e.preventDefault()
        onClose()
      }}
      onClick={(e) => {
        // Backdrop only: clicks inside the panel below stop here first.
        if (e.target === ref.current) onClose()
      }}
      className="mt-24 w-full max-w-xl rounded-pd border border-border bg-surface p-0 text-text backdrop:bg-bg/80"
    >
      <div className="p-3">
        <input
          autoFocus
          type="text"
          role="combobox"
          aria-expanded="true"
          aria-controls="palette-list"
          aria-activedescendant={shown[active] === undefined ? undefined : `palette-${shown[active].id}`}
          aria-label={t('palette.title')}
          placeholder={t('palette.placeholder')}
          value={query}
          onChange={(e) => {
            setQuery(e.target.value)
            setAt(0)
          }}
          onKeyDown={(e) => {
            if (e.key === 'ArrowDown') {
              e.preventDefault()
              setAt((i) => Math.min(i + 1, shown.length - 1))
            }
            if (e.key === 'ArrowUp') {
              e.preventDefault()
              setAt((i) => Math.max(i - 1, 0))
            }
            if (e.key === 'Enter') {
              e.preventDefault()
              dispatch(shown[active])
            }
          }}
          className="w-full rounded-pd border border-border bg-bg px-3 py-1.5 font-mono text-data"
        />

        <ul id="palette-list" role="listbox" aria-label={t('palette.title')} className="mt-2 max-h-80 overflow-auto">
          {shown.map((action, i) => (
            <li
              key={action.id}
              id={`palette-${action.id}`}
              role="option"
              aria-selected={i === active}
              data-action-id={action.id}
              onClick={() => {
                dispatch(action)
              }}
              className={`flex cursor-pointer items-baseline justify-between gap-3 rounded-pd px-3 py-1.5 text-data ${
                i === active ? 'bg-surface-2 text-accent' : 'text-text'
              }`}
            >
              <span className="min-w-0 truncate">{action.label}</span>
              <span className="shrink-0 text-text-dim">{action.group}</span>
            </li>
          ))}
        </ul>

        {shown.length === 0 ? (
          <p className="px-3 py-2 font-mono text-data text-text-dim">
            {`▸ ${t('palette.noMatch', { query })}`}
          </p>
        ) : null}
      </div>
    </dialog>
  )
}
