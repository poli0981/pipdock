/**
 * Installed and Updates — UI-SPEC §4.
 *
 * One screen with two modes, because §4 defines Updates as "the same table filtered to outdated"
 * and DATA-FLOW §4 has outdated rows "mirrored into the Updates tab". Both read the same joined
 * array from the store, so a package cannot be outdated in one tab and not the other, and there is
 * one scan feeding both.
 *
 * Loading is automatic on environment selection, not behind a button: UI-SPEC §5's click budget
 * spends that click already ("app auto-scans on env open").
 */

import { useCallback, useEffect } from 'react'
import { useTranslation } from 'react-i18next'

import { PdEmptyState } from '@/components/PdEmptyState'
import { PdErrorRow } from '@/components/PdErrorRow'
import { PdPackageTable } from '@/components/PdPackageTable'
import { outdatedOnly, selectableForUpdate } from '@/screens/rows'
import { useEnvStore } from '@/stores'

interface PdPackagesProps {
  mode: 'installed' | 'updates'
}

export function PdPackages({ mode }: PdPackagesProps) {
  const { t } = useTranslation()

  // Per-field selectors throughout. Destructuring the whole store re-renders a 200-row
  // virtualized table on every unrelated field change, including each `scan-progress` tick.
  const selected = useEnvStore((s) => s.selected)
  const rows = useEnvStore((s) => s.rows)
  const packages = useEnvStore((s) => s.packages)
  const orphanOutdated = useEnvStore((s) => s.orphanOutdated)
  const listing = useEnvStore((s) => s.listing)
  const listError = useEnvStore((s) => s.listError)
  const outdatedStatus = useEnvStore((s) => s.outdatedStatus)
  const outdatedError = useEnvStore((s) => s.outdatedError)
  const selection = useEnvStore((s) => s.selection)
  const loadedFor = useEnvStore((s) => s.loadedFor)
  const loadPackages = useEnvStore((s) => s.loadPackages)
  const loadOutdated = useEnvStore((s) => s.loadOutdated)
  const toggle = useEnvStore((s) => s.toggle)
  const selectAll = useEnvStore((s) => s.selectAll)
  const clearSelection = useEnvStore((s) => s.clearSelection)
  const togglePin = useEnvStore((s) => s.togglePin)

  // The row's handler must return void. Wrapped here rather than in the store so `togglePin` stays
  // awaitable for the tests, and stable across renders so the memoized rows are not defeated.
  const pinToggle = useCallback(
    (name: string) => {
      void togglePin(name)
    },
    [togglePin],
  )

  useEffect(() => {
    // Guarded on `loadedFor` so switching between Installed and Updates does not refetch, and so
    // the two screens mounting in turn do not both start a scan.
    if (selected !== null && loadedFor !== selected) {
      void loadPackages()
      void loadOutdated()
    }
  }, [selected, loadedFor, loadPackages, loadOutdated])

  const row = rows.find((r) => r.interpreter === selected)
  const visible = mode === 'updates' ? outdatedOnly(packages) : packages
  const { selectable, pinnedExcluded } = selectableForUpdate(visible)
  const title = mode === 'updates' ? t('packages.updatesTitle') : t('packages.installedTitle')

  if (selected === null) {
    return (
      <section aria-labelledby="pkg-title" className="h-full overflow-auto p-6">
        <h1 id="pkg-title" className="text-accent">
          {title}
        </h1>
        <PdEmptyState
          message={t('packages.noEnvironment')}
          hint={t('packages.noEnvironmentHint')}
        />
      </section>
    )
  }

  return (
    <section aria-labelledby="pkg-title" className="flex h-full min-h-0 flex-col p-6">
      <div className="flex shrink-0 items-center justify-between gap-3">
        <h1 id="pkg-title" className="text-accent">
          {title}
        </h1>
        <div className="flex items-center gap-2 text-data text-text-dim">
          {outdatedStatus === 'loading' ? <span>{t('packages.checkingUpdates')}</span> : null}
          {selection.size > 0 ? (
            <>
              <span>{t('packages.selected', { count: selection.size })}</span>
              <button
                type="button"
                onClick={clearSelection}
                className="rounded-pd border border-border px-2 py-0.5"
              >
                {t('packages.actions.clearSelection')}
              </button>
            </>
          ) : null}
          <button
            type="button"
            onClick={() => {
              selectAll(selectable)
            }}
            disabled={selectable.length === 0}
            className="rounded-pd border border-border px-3 py-1 disabled:opacity-40"
          >
            {t('actions.selectAll')}
          </button>
          {/* S3's entry point. Rendered disabled rather than omitted, so the 4-click budget in
              UI-SPEC §5 is honest about where the click will be when the preview lands. */}
          <button
            type="button"
            disabled
            title={t('packages.updateSelectedSoon')}
            className="rounded-pd border border-border px-3 py-1 disabled:opacity-40"
          >
            {t('packages.actions.updateSelected')}
          </button>
        </div>
      </div>

      {/* UI-SPEC §4: pinned rows are excluded from Select all, and it says so. Presentation only —
          DATA-FLOW §9.5 is enforced by `pins::filter_upgrades` when a plan is built (S3). */}
      {pinnedExcluded > 0 ? (
        <p className="mt-1 shrink-0 text-data text-info">
          {t('packages.pinnedExcluded', { count: pinnedExcluded })}
        </p>
      ) : null}

      {/* SECURITY §2 and SP-6: only when the probe really hid something, and it names the path. */}
      {row?.env?.hiddenUserSite != null ? (
        <p className="mt-2 shrink-0 rounded-pd border-l-2 border-info pl-2 text-data text-text-dim">
          {t('env.partialListing')} {t('env.partialListingDetail', { path: row.env.hiddenUserSite })}
        </p>
      ) : null}

      {/* The two sources can disagree about what is installed — the probe runs isolated and hides
          user-site packages, `pip list --outdated` does not. Saying so beats a badge count that
          promises rows the table cannot show. */}
      {orphanOutdated.length > 0 ? (
        <p className="mt-2 shrink-0 text-data text-text-dim">
          {t('packages.orphanOutdated', { count: orphanOutdated.length })}
        </p>
      ) : null}

      {listError !== null ? (
        <div className="mt-4 shrink-0">
          <PdErrorRow error={listError} />
        </div>
      ) : null}

      {/* Scoped to the outdated fetch: the table below stays. */}
      {outdatedError !== null ? (
        <div className="mt-4 shrink-0">
          <p className="mb-1 text-data text-text-dim">{t('packages.outdatedFailed')}</p>
          <PdErrorRow error={outdatedError} />
        </div>
      ) : null}

      {listing === 'loading' ? (
        <p aria-live="polite" className="mt-4 shrink-0 text-text-dim">
          {t('packages.loading')}
        </p>
      ) : null}

      {listing === 'ready' && visible.length === 0 ? (
        <PdEmptyState
          message={mode === 'updates' ? t('packages.updatesEmpty') : t('packages.empty')}
          {...(mode === 'updates' ? {} : { hint: t('packages.emptyHint') })}
        />
      ) : null}

      {visible.length > 0 ? (
        <div className="mt-4 flex min-h-0 flex-1 flex-col">
          <PdPackageTable
            rows={visible}
            outdatedStatus={outdatedStatus}
            selection={selection}
            onToggle={toggle}
            onPinToggle={pinToggle}
            onSelectAll={() => {
              selectAll(selectable)
            }}
          />
        </div>
      ) : null}
    </section>
  )
}
