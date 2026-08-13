/**
 * Code Health — UI-SPEC §4's one sentence, CODE-HEALTH-SPEC §5's run header and tabs.
 *
 * The screen owns the folder, the run, the progress and the errors; `PdHealthReport` owns the
 * three tabs and is given a report. The division is the one `PdPlanPanel`/`PdPreviewDiff` uses,
 * and it is what lets the tabs be tested against a fixture with no store at all.
 *
 * **The folder is remembered per environment, and it arrives on the row.** `EnvRow.healthProject`
 * is filled by `env_scan` and `env_probe` from the same `health_projects` table the CLI writes,
 * so the second run in a project costs one click fewer than the first — which is the difference
 * between UI-SPEC §5's budget of 3 and the 4 a first run actually takes.
 */

import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { PdConsoleDrawer } from '@/components/PdConsoleDrawer'
import { PdDialog } from '@/components/PdDialog'
import { PdEmptyState } from '@/components/PdEmptyState'
import { PdErrorRow } from '@/components/PdErrorRow'
import { PdHealthReport } from '@/components/PdHealthReport'
import { healthSaveReport, pickProjectFolder, pickSavePath } from '@/ipc'
import type { PdError, ToolProblem } from '@/ipc'
import { useEnvPackages } from '@/screens/useEnvPackages'
import { freshReport, useEnvStore, useHealthStore, usePlanStore } from '@/stores'

/**
 * A failed tool as an error row.
 *
 * `ToolProblem` is `PdError`-shaped on purpose, but not identically typed: `exactOptionalPropertyTypes`
 * is on and the generator emits `?: T | null` for every `Option<T>`, so an absent `stderrTail` and
 * a null one are different types and neither is `string`. Same conditional spread
 * `PdSummarySheet` uses.
 */
function asError(problem: ToolProblem): PdError {
  return {
    code: problem.code,
    message: problem.message,
    ...(problem.stderrTail == null ? {} : { stderrTail: problem.stderrTail }),
  }
}

export function PdHealth() {
  const { t } = useTranslation()

  const selected = useEnvStore((s) => s.selected)
  const rows = useEnvStore((s) => s.rows)
  const dists = useEnvStore((s) => s.dists)
  const startUninstall = usePlanStore((s) => s.startUninstall)

  // Per-field selectors throughout, for `PdPackages`' reason: destructuring the store re-renders
  // a report of several hundred rows on every unrelated field change, including each progress tick.
  const folder = useHealthStore((s) => s.folder)
  const report = useHealthStore((s) => s.report)
  const reportFor = useHealthStore((s) => s.reportFor)
  const ruffByFile = useHealthStore((s) => s.ruffByFile)
  const tab = useHealthStore((s) => s.tab)
  const phase = useHealthStore((s) => s.phase)
  const error = useHealthStore((s) => s.error)
  const consoleLines = useHealthStore((s) => s.console)
  const done = useHealthStore((s) => s.done)
  const total = useHealthStore((s) => s.total)
  const consoleOpen = useHealthStore((s) => s.consoleOpen)
  const setFolder = useHealthStore((s) => s.setFolder)
  const setTab = useHealthStore((s) => s.setTab)
  const setConsoleOpen = useHealthStore((s) => s.setConsoleOpen)
  const run = useHealthStore((s) => s.run)
  const fix = useHealthStore((s) => s.fix)
  const fixBusy = useHealthStore((s) => s.fixBusy)
  const fixError = useHealthStore((s) => s.fixError)
  const openFix = useHealthStore((s) => s.openFix)
  const closeFix = useHealthStore((s) => s.closeFix)
  const confirmFix = useHealthStore((s) => s.confirmFix)

  const row = rows.find((r) => r.interpreter === selected)
  // Hoisted, for `PdPackages`' reason: a narrowing that has to be re-derived inside a closure is
  // one the compiler cannot carry, and `row?.env` is exactly that shape.
  const env = row?.env

  // `dists` belongs to the package slice and is only filled once the user has visited Installed
  // or Updates. Without this the deptry handoff's button would appear or vanish depending on
  // which tab they happened to open first — "never render a state you have not loaded", in
  // reverse. The hook guards on `loadedFor` and does not refetch.
  useEnvPackages()

  // The remembered folder, taken from the row the moment one is known. Not a fallback inside the
  // render: `setFolder` resets the report when the folder actually changes, and calling it during
  // render would reset on every pass.
  useEffect(() => {
    if (row?.healthProject !== undefined && folder === null) setFolder(row.healthProject)
  }, [row?.healthProject, folder, setFolder])

  const running = phase === 'running'
  // Only when it describes *this* environment and folder. A plain `report` read would show one
  // environment's findings after the user switched to another.
  const shown = row === undefined ? null : freshReport({ report, reportFor, folder }, row.envHash)

  // Where the last save went, so the confirmation names the files rather than just claiming
  // success. Screen state, not store state: it describes an action, not a report.
  const [saved, setSaved] = useState<string[] | null>(null)

  const choose = () => {
    void pickProjectFolder(t('health.pickerTitle'), folder ?? undefined).then((chosen) => {
      if (chosen !== null) setFolder(chosen)
    })
  }

  return (
    <section aria-labelledby="health-title" className="flex h-full min-h-0 flex-col p-6">
      <h1 id="health-title" className="shrink-0 text-accent">
        {t('health.title')}
      </h1>

      {row === undefined ? (
        <PdEmptyState
          message={t('health.noEnvironment')}
          hint={t('health.noEnvironmentHint')}
        />
      ) : (
        <>
          <div className="mt-4 flex shrink-0 flex-wrap items-center gap-3">
            {/* A path is data and is never localized (I18N §2). */}
            <code className="min-w-0 flex-1 truncate font-mono text-data text-text-dim">
              {folder ?? t('health.noFolder')}
            </code>
            <button
              type="button"
              className="rounded-pd border border-border px-2 py-1 text-data hover:bg-surface-2"
              disabled={running}
              onClick={choose}
            >
              {folder === null ? t('health.chooseFolder') : t('health.changeFolder')}
            </button>
            <button
              type="button"
              data-action="run-health"
              className="rounded-pd bg-accent px-3 py-1 text-data text-bg disabled:opacity-50"
              disabled={running || folder === null || env === undefined}
              onClick={() => {
                if (env !== undefined) void run(env, row.envHash)
              }}
            >
              {running ? t('health.running') : t('health.run')}
            </button>
            <button
              type="button"
              className="rounded-pd border border-border px-2 py-1 text-data hover:bg-surface-2 disabled:opacity-50"
              disabled={shown === null}
              onClick={() => {
                if (shown === null) return
                void pickSavePath(t('health.saveTitle'), 'code-health.md').then((target) => {
                  if (target === null) return
                  void healthSaveReport(shown, target).then(setSaved)
                })
              }}
            >
              {t('health.save')}
            </button>
            <button
              type="button"
              className="rounded-pd border border-border px-2 py-1 text-data text-text-dim hover:bg-surface-2"
              onClick={() => {
                setConsoleOpen(!consoleOpen)
              }}
            >
              {t('health.log')}
            </button>
          </div>

          {/* One live region for the screen: progress while running, then where a save landed.
              Two regions would serialize against each other unpredictably. */}
          <p aria-live="polite" className="mt-2 shrink-0 text-data text-text-dim">
            {running && total > 0 ? t('plan.progress', { done, total }) : null}
            {/* Paths are data and are never localized (I18N §2). */}
            {!running && saved !== null ? t('health.saved', { files: saved.join(', ') }) : null}
          </p>

          {/* A run that failed outright — distinct from a *tool* that failed, which is below. */}
          {error !== null ? (
            <div className="mt-2 shrink-0">
              <PdErrorRow error={error} />
            </div>
          ) : null}

          {/* A refused or failed fix. Separate from `error`, which is a run that failed: this one
              leaves the report on screen, because it is still true. */}
          {fixError !== null ? (
            <div className="mt-2 shrink-0">
              <PdErrorRow error={fixError} />
            </div>
          ) : null}

          {/* Above the tabs, and counted into `⚠ n`. A failed tool is a problem currently on
              screen, and there are at most three of them — this is not `PdSummarySheet`'s
              one-row-per-package case. Above rather than inside a tab, because the explanation
              must not be reachable only from the tab it explains. */}
          {(shown?.problems ?? []).map((problem) => (
            <div key={problem.tool} className="mt-2 shrink-0">
              <PdErrorRow error={asError(problem)} />
            </div>
          ))}

          {shown === null ? (
            // **Never "no issues found" here.** No run has happened, so there is nothing to have
            // found — saying otherwise is the same lie a tab keyed on `ran` alone tells about a
            // failed tool, one level up. Found by opening the screen with a remembered folder and
            // reading what it said before anything ran.
            <PdEmptyState
              message={folder === null ? t('health.noFolder') : t('health.notRunYet')}
              hint={folder === null ? t('health.noFolderHint') : t('health.notRunYetHint')}
            />
          ) : (
            <div className="mt-4 flex min-h-0 flex-1 flex-col">
              <p className="shrink-0 text-data text-text-dim">
                {t('health.checkedAt', { when: shown.ranAt })}
              </p>
              <PdHealthReport
                report={shown}
                ruffByFile={ruffByFile}
                tab={tab}
                onTab={setTab}
                installed={dists.map((d) => d.name)}
                onFix={() => {
                  void openFix()
                }}
                onUninstall={
                  env === undefined
                    ? undefined
                    : (distribution) => {
                        // `PdUninstallDialog` is mounted outside `<main>`, so this opens over
                        // Health with no navigation. On confirm the plan panel takes the content
                        // area; the report survives because it lives in the store.
                        void startUninstall(env, [distribution])
                      }
                }
              />
            </div>
          )}
        </>
      )}

      {/* One dialog with two states, not two dialogs: two confirms would break the click budget
          and the second is the one nobody reads. Cancel is rendered first and focused by
          `PdDialog`, so Enter without Tab cancels — UI-SPEC §7's rule for a destructive confirm. */}
      {fix !== null ? (
        <PdDialog
          label={t('health.fixTitle')}
          title={t('health.fixTitle')}
          cancelLabel={t('actions.cancel')}
          busy={fixBusy}
          onCancel={closeFix}
          actions={
            <button
              type="button"
              data-action="fix"
              className="rounded-pd bg-danger px-3 py-1 text-data text-bg disabled:opacity-50"
              disabled={fixBusy}
              onClick={() => {
                void confirmFix()
              }}
            >
              {fix.dirty === null ? t('health.fixConfirm') : t('health.fixConfirmDirty')}
            </button>
          }
        >
          <p className="text-data">{t('health.fixBody', { count: fix.files })}</p>
          {/* Unconditional, and true in every case — including no repository at all, which is
              deliberately not treated as a warning of its own. */}
          <p className="mt-2 text-data text-text-dim">{t('health.fixCannotUndo')}</p>
          {fix.dirty === null ? null : (
            <p className="mt-2 border-l-2 border-danger pl-2 text-data text-danger">
              {t('health.fixDirty', { count: fix.dirty })}
            </p>
          )}
        </PdDialog>
      ) : null}

      <PdConsoleDrawer
        lines={consoleLines}
        open={consoleOpen}
        done={done}
        total={total}
        onClose={() => {
          setConsoleOpen(false)
        }}
      />
    </section>
  )
}
