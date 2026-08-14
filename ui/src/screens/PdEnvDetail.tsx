/**
 * One environment, in detail — UI-SPEC §4 puts Snapshots here: "surfaced under Environments → env
 * detail".
 *
 * It is a *mode* of the Environments tab rather than a sidebar entry of its own. Appending is free
 * — Phase 4 put About on the end as `Ctrl+9` without moving a binding — but Snapshots would have to
 * sit beside Environments to read as related, and an *insert* renumbers everything after it, which
 * the M2 plan is explicit about not doing. `useEnvStore`
 * holds which environment is open, not this component's state: the plan panel replaces the whole
 * content area while a rollback runs, so local state would be unmounted with it and the user would
 * land back on the flat list the moment their rollback finished.
 *
 * A row whose interpreter is gone still gets here. Snapshots are keyed by `envHash` and outlive the
 * Python that made them, so the timeline lists — but nothing can be diffed or restored against an
 * interpreter that cannot be frozen, and the screen says which rather than spinning.
 */

import { useTranslation } from 'react-i18next'

import { PdEmptyState } from '@/components/PdEmptyState'
import { PdErrorRow } from '@/components/PdErrorRow'
import { PdSnapshotTimeline } from '@/components/PdSnapshotTimeline'
import { useEnvStore, usePlanStore } from '@/stores'
import { useEnvSnapshots } from '@/screens/useEnvSnapshots'

export function PdEnvDetail() {
  const { t } = useTranslation()
  const openFor = useEnvStore((s) => s.openFor)
  const rows = useEnvStore((s) => s.rows)
  const snapshots = useEnvStore((s) => s.snapshots)
  const snapshotsLoading = useEnvStore((s) => s.snapshotsLoading)
  const snapshotsError = useEnvStore((s) => s.snapshotsError)
  const selectedSnapshot = useEnvStore((s) => s.selectedSnapshot)
  const diff = useEnvStore((s) => s.diff)
  const diffLoading = useEnvStore((s) => s.diffLoading)
  const closeEnv = useEnvStore((s) => s.closeEnv)
  const selectSnapshot = useEnvStore((s) => s.selectSnapshot)
  const takeSnapshot = useEnvStore((s) => s.takeSnapshot)
  const rollback = usePlanStore((s) => s.rollback)

  useEnvSnapshots()

  const row = rows.find((r) => r.interpreter === openFor)
  const env = row?.env
  const usable = env !== undefined

  return (
    <section aria-labelledby="detail-title" className="h-full overflow-auto p-6">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h1 id="detail-title" className="min-w-0 truncate text-accent">
          {t('snapshots.title')}
        </h1>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => {
              void takeSnapshot()
            }}
            disabled={!usable}
            data-action="take-snapshot"
            className="rounded-pd border border-border px-3 py-1 text-data disabled:opacity-40"
          >
            {t('snapshots.take')}
          </button>
          <button
            type="button"
            onClick={closeEnv}
            className="rounded-pd border border-border px-3 py-1 text-data"
          >
            {t('actions.back')}
          </button>
        </div>
      </div>

      <p className="mt-1 font-mono text-data text-text-dim">{openFor}</p>

      {usable ? null : (
        <p className="mt-2 rounded-pd border-l-2 border-warn pl-2 text-data text-text-dim">
          {t('snapshots.envUnusable')}
        </p>
      )}

      {snapshotsError !== null ? (
        <div className="mt-4">
          <PdErrorRow error={snapshotsError} />
        </div>
      ) : null}

      {/* **One live region for the screen**, not one per loading state. Two polite regions on the
          same screen serialize in an order neither of them controls, so a reader can hear the
          older message after the newer one. This one covers both waits; the snapshot list and a
          diff are never loading at the same moment, because a diff needs a selection from the
          list. */}
      <p aria-live="polite" className="mt-4 text-data text-text-dim">
        {snapshotsLoading === 'loading' ? t('snapshots.loading') : null}
        {diffLoading === 'loading' ? t('snapshots.diffing') : null}
      </p>

      {snapshotsLoading === 'ready' && snapshots.length === 0 ? (
        <PdEmptyState message={t('snapshots.empty')} hint={t('snapshots.emptyHint')} />
      ) : (
        <PdSnapshotTimeline
          snapshots={snapshots}
          selected={selectedSnapshot}
          selectable={usable}
          onSelect={(id) => {
            void selectSnapshot(id)
          }}
        />
      )}

      {selectedSnapshot !== null && usable ? (
        <div className="mt-6 border-t border-border pt-4">


          {diff !== null ? (
            <>
              <DiffView />
              <button
                type="button"
                onClick={() => {
                  void rollback(env, selectedSnapshot)
                }}
                data-action="rollback"
                className="mt-4 rounded-pd border border-danger px-3 py-1 text-data text-danger"
              >
                {t('snapshots.rollback')}
              </button>
            </>
          ) : null}
        </div>
      ) : null}
    </section>
  )
}

/** The diff viewer — UI-SPEC §4: "added/removed/changed in mono". */
function DiffView() {
  const { t } = useTranslation()
  const diff = useEnvStore((s) => s.diff)
  if (diff === null) return null

  const empty =
    diff.added.length === 0 && diff.removed.length === 0 && diff.changed.length === 0
  if (empty) return <p className="text-data text-text-dim">{t('snapshots.noDifference')}</p>

  return (
    <dl className="space-y-3" data-diff>
      {/* "Added" means present now and absent from the snapshot — so restoring *removes* it. The
          copy says which direction, because the bare word is ambiguous the moment you are looking
          at it from the snapshot's side. */}
      <Group label={t('snapshots.diffAdded', { count: diff.added.length })} tone="warn">
        {diff.added.map((s) => `${s.name}==${s.version}`)}
      </Group>
      <Group label={t('snapshots.diffRemoved', { count: diff.removed.length })} tone="accent">
        {diff.removed.map((s) => `${s.name}==${s.version}`)}
      </Group>
      <Group label={t('snapshots.diffChanged', { count: diff.changed.length })} tone="info">
        {diff.changed.map((c) => `${c.name} ${c.current} → ${c.snapshot}`)}
      </Group>
    </dl>
  )
}

function Group({
  label,
  tone,
  children,
}: {
  label: string
  tone: 'warn' | 'accent' | 'info'
  children: string[]
}) {
  if (children.length === 0) return null
  const color = tone === 'warn' ? 'text-warn' : tone === 'accent' ? 'text-accent' : 'text-info'
  return (
    <div>
      <dt className={`text-data ${color}`}>{label}</dt>
      {children.map((line) => (
        <dd key={line}>
          <code className="font-mono text-data">{line}</code>
        </dd>
      ))}
    </div>
  )
}
