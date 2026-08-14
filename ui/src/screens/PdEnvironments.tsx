/**
 * Environments — UI-SPEC §4.
 *
 * The first real screen, and the one that exercises the whole bridge without any mutation risk:
 * multi-process discovery with live progress, three error codes, the PEP 668 `MANAGED` chip, the
 * `hidden_user_site` partial-listing note, an empty state, and selection feeding the status line.
 *
 * A row whose probe failed renders **in place** with its code rather than disappearing: one broken
 * interpreter must not hide the rest, which is the rule `pipdock env list` already follows.
 */

import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { PdDialog } from '@/components/PdDialog'
import { PdEmptyState } from '@/components/PdEmptyState'
import { PdErrorRow } from '@/components/PdErrorRow'
import { REPO } from '@/components/legal'
import { useOpenExternal } from '@/components/useOpenExternal'
import { onScanProgress, type EnvRow } from '@/ipc'
import { PdEnvDetail } from '@/screens/PdEnvDetail'
import { PIP_MIN_FOR_REPORT, pipNeedsUpgrade } from '@/screens/pip'
import { useEnvStore } from '@/stores'

/** `22.2`, for the dialog's copy. Data, not prose — interpolated, never translated (I18N §2). */
const FLOOR = PIP_MIN_FOR_REPORT.join('.')

function SourceChip({ row }: { row: EnvRow }) {
  const { t } = useTranslation()
  return (
    <span className="rounded-pd border border-border px-1.5 py-0.5 text-data text-text-dim">
      {t(`env.source.${row.source}`)}
    </span>
  )
}

function Row({ row }: { row: EnvRow }) {
  const { t } = useTranslation()
  const selected = useEnvStore((s) => s.selected)
  const select = useEnvStore((s) => s.select)
  const openEnv = useEnvStore((s) => s.openEnv)
  const upgradePip = useEnvStore((s) => s.upgradePip)
  const upgrading = useEnvStore((s) => s.upgradingPip === row.interpreter)
  const [confirming, setConfirming] = useState(false)
  const isSelected = selected === row.interpreter
  const managed = row.env?.externallyManaged === true

  return (
    <li
      className={`rounded-pd border p-3 ${
        isSelected ? 'border-accent bg-surface-2' : 'border-border bg-surface'
      }`}
    >
      <div className="flex flex-wrap items-center gap-2">
        <code className="font-mono text-data">{row.interpreter}</code>
        <SourceChip row={row} />
        {managed ? (
          <span className="rounded-pd bg-danger/20 px-1.5 py-0.5 text-data text-danger">
            {t('env.managed')}
          </span>
        ) : null}
        {isSelected ? (
          <span className="text-data text-accent">{t('env.selected')}</span>
        ) : null}
      </div>

      {row.env !== undefined ? (
        <p className="mt-1 font-mono text-data text-text-dim">
          {/* Version and counts are data, not prose — never translated (I18N §2). */}
          {`Py ${row.env.pythonVersion}`}
          {row.packages === undefined ? '' : ` · ${t('status.packages', { count: row.packages })}`}
          {row.pipVersion === undefined ? '' : ` · pip ${row.pipVersion}`}
        </p>
      ) : null}

      {/* SECURITY §2 and SP-6: shown only when the probe actually hid something, and it names the
          path. Informational, never a block. */}
      {row.env?.hiddenUserSite != null ? (
        <p className="mt-2 rounded-pd border-l-2 border-info pl-2 text-data text-text-dim">
          {t('env.partialListing')}{' '}
          {t('env.partialListingDetail', { path: row.env.hiddenUserSite })}
        </p>
      ) : null}

      {managed ? (
        <p className="mt-2 text-data text-text-dim">{t('env.managedDetail')}</p>
      ) : null}

      {row.error !== undefined ? (
        <div className="mt-2">
          <PdErrorRow error={row.error} />
        </div>
      ) : (
        <div className="mt-2 flex items-center gap-2">
          <button
            type="button"
            onClick={() => {
              select(row.interpreter)
            }}
            disabled={isSelected}
            className="rounded-pd border border-border px-3 py-1 text-data disabled:opacity-40"
          >
            {t('actions.use')}
          </button>
          {/* UI-SPEC §5's 4-click rollback starts here: Environments is the landing screen, so
              Open → snapshot → Rollback… → Roll back is the whole budget. */}
          <button
            type="button"
            onClick={() => {
              openEnv(row.interpreter)
            }}
            data-action="open"
            className="rounded-pd border border-border px-3 py-1 text-data"
          >
            {t('actions.open')}
          </button>
          {/* P0-10. A second inline button rather than a `⋮` menu: there is no row overflow menu
              anywhere in the app, and the first one should not arrive to hold a single action.
              Shown only below the planner's floor — see `pipNeedsUpgrade`. */}
          {pipNeedsUpgrade(row.pipVersion) ? (
            <button
              type="button"
              onClick={() => {
                setConfirming(true)
              }}
              disabled={upgrading}
              data-action="upgrade-pip"
              className="rounded-pd border border-warn px-3 py-1 text-data text-warn disabled:opacity-40"
            >
              {upgrading ? t('env.pipUpgrading') : t('env.pipUpgrade')}
            </button>
          ) : null}
        </div>
      )}

      {confirming ? (
        <PdDialog
          label={row.interpreter}
          title={t('env.pipUpgradeTitle')}
          cancelLabel={t('actions.cancel')}
          busy={upgrading}
          onCancel={() => {
            setConfirming(false)
          }}
          actions={
            <button
              type="button"
              onClick={() => {
                setConfirming(false)
                void upgradePip(row.interpreter)
              }}
              data-action="upgrade-pip"
              className="rounded-pd border border-warn px-3 py-1 text-data text-warn"
            >
              {t('env.pipUpgradeConfirm')}
            </button>
          }
        >
          <p className="text-data text-text-dim">
            {t('env.pipUpgradeBody', { version: row.pipVersion ?? '', floor: FLOOR })}
          </p>
          {/* DATA-FLOW §9.2's exemption, said out loud. A snapshot's only restore path is
              `pip install pip==X` run *by pip*, so one taken here would have no consumer if pip
              broke — but an unannounced exception to "every mutation is snapshotted" is how an
              exemption becomes an erosion. */}
          <p className="mt-2 text-data text-warn">{t('env.pipUpgradeNoSnapshot')}</p>
        </PdDialog>
      ) : null}
    </li>
  )
}

export function PdEnvironments() {
  const { t } = useTranslation()
  // Per-field selectors, not a bare `useEnvStore()`: that subscribes to every field, so opening
  // a detail view or selecting a snapshot would re-render the whole environment list.
  const rows = useEnvStore((s) => s.rows)
  const scanning = useEnvStore((s) => s.scanning)
  const progress = useEnvStore((s) => s.progress)
  const error = useEnvStore((s) => s.error)
  const scan = useEnvStore((s) => s.scan)
  const setProgress = useEnvStore((s) => s.setProgress)
  const openFor = useEnvStore((s) => s.openFor)
  const { open, failed: openFailed } = useOpenExternal()

  useEffect(() => {
    // Subscribe before scanning, or the first phases are emitted into nothing.
    let unlisten: (() => void) | undefined
    void onScanProgress(setProgress).then((fn) => {
      unlisten = fn
    })
    void scan()
    return () => {
      unlisten?.()
    }
  }, [scan, setProgress])

  // The detail view is a mode of this tab, not a sidebar entry of its own. Not because the sidebar
  // is full — Phase 4 appended About as the ninth and nothing renumbered — but because Snapshots
  // would have to sit *beside* Environments to read as related, and that is an insert, which moves
  // every shortcut after it. Below the hooks, so the hook order is identical either way.
  if (openFor !== null) return <PdEnvDetail />

  return (
    <section aria-labelledby="env-title" className="h-full overflow-auto p-6">
      <div className="flex items-center justify-between">
        <h1 id="env-title" className="text-accent">
          {t('env.title')}
        </h1>
        <button
          type="button"
          onClick={() => {
            void scan()
          }}
          disabled={scanning}
          className="rounded-pd border border-border px-3 py-1 text-data disabled:opacity-40"
        >
          {t('actions.rescan')}
        </button>
      </div>

      {scanning ? (
        <p aria-live="polite" className="mt-4 text-text-dim">
          {progress === null
            ? t('env.scanning')
            : `${t(`env.scanPhase.${progress.phase}`)} (${progress.done}/${progress.total})`}
        </p>
      ) : null}

      {error !== null ? (
        <div className="mt-4">
          <PdErrorRow error={error} />
        </div>
      ) : null}

      {!scanning && rows.length === 0 && error === null ? (
        <PdEmptyState message={t('env.empty')} hint={t('env.emptyHint')} />
      ) : null}

      <ul className="mt-4 space-y-2">
        {rows.map((row) => (
          <Row key={row.interpreter} row={row} />
        ))}
      </ul>

      <p className="mt-6 text-data text-text-dim">
        <button
          type="button"
          onClick={() => {
            open(REPO)
          }}
          className="text-accent-dim underline underline-offset-2"
        >
          {t('app.name')}
        </button>
      </p>

      {openFailed ? (
        <p className="mt-2 text-data text-warn" role="alert">
          {t('actions.openFailed')}
        </p>
      ) : null}
    </section>
  )
}
