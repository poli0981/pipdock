/**
 * The dependency view — a **mode of the package screen**, not a tenth tab (PRD P1-6, UI-SPEC §4).
 *
 * `PdEnvironments` switches to `PdEnvDetail` on `openFor`; this switches on `focus`, for the same
 * reason UI-SPEC §4 gives for Snapshots: `Ctrl+1..9` is positional over `NAV_KEYS`, so appending
 * is free and **inserting is not**. A Dependencies tab would have to sit beside Installed to read
 * as related, which renumbers every shortcut after it — and `NAV_KEYS` is already nine, so a tenth
 * entry has no `Ctrl+` digit at all (`App.tsx` reads `NAV_KEYS[key - 1]`, and `Ctrl+0` is index
 * `-1`).
 *
 * Which package is focused lives in `useDepsStore`, not in component state, for `PdEnvDetail`'s
 * reason: the plan panel replaces the whole content area while a run is in flight, so local state
 * would be unmounted with it and the user would land back on the flat table the moment their
 * install finished.
 */

import { useEffect } from 'react'
import { useTranslation } from 'react-i18next'

import { PdDepsFocus } from '@/components/PdDepsFocus'
import { PdErrorRow } from '@/components/PdErrorRow'
import { freshGraph, nodeOf, useDepsStore, useEnvStore } from '@/stores'

export function PdDeps() {
  const { t } = useTranslation()

  // Per-field selectors, as everywhere else: destructuring re-renders on every unrelated change.
  const selected = useEnvStore((s) => s.selected)
  const rows = useEnvStore((s) => s.rows)
  const focus = useDepsStore((s) => s.focus)
  const graph = useDepsStore((s) => s.graph)
  const graphFor = useDepsStore((s) => s.graphFor)
  const phase = useDepsStore((s) => s.phase)
  const error = useDepsStore((s) => s.error)
  const load = useDepsStore((s) => s.load)
  const refocus = useDepsStore((s) => s.refocus)
  const close = useDepsStore((s) => s.close)

  const row = rows.find((r) => r.interpreter === selected)
  const envHash = row?.envHash

  useEffect(() => {
    if (row?.env === undefined || envHash === undefined) return
    void load(row.env, envHash)
  }, [load, row?.env, envHash])

  // Only when it describes *this* environment. A plain `graph` read would show one environment's
  // edges after the user switched to another.
  const shown = envHash === undefined ? null : freshGraph({ graph, graphFor }, envHash)
  const node = nodeOf(shown, focus)

  return (
    <section aria-labelledby="deps-title" className="h-full overflow-auto p-6">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h1 id="deps-title" className="min-w-0 truncate text-accent">
          {t('deps.title')}
        </h1>
        <button
          type="button"
          onClick={close}
          className="rounded-pd border border-border px-3 py-1 text-data"
        >
          {t('deps.back')}
        </button>
      </div>

      {error === null ? null : (
        <div className="mt-4">
          <PdErrorRow error={error} />
        </div>
      )}

      <div className="mt-4">
        {/* Three states, and the middle one is the one P4 got wrong in the other direction: a
            screen that says "nothing found" while a fetch is in flight is a lie, and so is one
            that says "loading" over a graph it already has. `phase` decides, not the data. */}
        {phase === 'loading' && shown === null ? (
          <p className="text-data text-text-dim">{t('deps.loading')}</p>
        ) : focus === null ? null : (
          <PdDepsFocus pkg={focus} node={node} onFocus={refocus} />
        )}
      </div>
    </section>
  )
}
