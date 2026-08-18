/**
 * Security — PRD P1-1, SECURITY §6, UI-SPEC §3's seventh tab.
 *
 * The screen owns the run, the errors and the progress; `PdAuditReport` owns the list. Same
 * division as `PdHealth`/`PdHealthReport`, and it is what lets the list be tested against a
 * generated fixture with no store at all.
 *
 * **Never says "no advisories" before a run.** The screen routes an un-run environment to its own
 * empty state and only hands a *report* to `PdAuditReport`, because "nothing was found" and
 * "nothing has run" are different claims and P4 shipped the wrong one once already.
 */

import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { PdAuditReport } from '@/components/PdAuditReport'
import { PdConsoleDrawer } from '@/components/PdConsoleDrawer'
import { PdEmptyState } from '@/components/PdEmptyState'
import { PdErrorRow } from '@/components/PdErrorRow'
import { auditSaveReport, pickSavePath } from '@/ipc'
import type { PdError, ToolProblem } from '@/ipc'
import { freshAudit, useEnvStore, useSecurityStore } from '@/stores'

/** A `ToolProblem` is `PdError`-shaped on purpose, so the row takes it directly. */
function asError(problem: ToolProblem): PdError {
  return {
    code: problem.code,
    message: problem.message,
    ...(problem.stderrTail == null ? {} : { stderrTail: problem.stderrTail }),
  }
}

export function PdSecurity() {
  const { t } = useTranslation()

  // Per-field selectors throughout, for `PdHealth`'s reason: destructuring the store re-renders a
  // list of several hundred rows on every unrelated field change, including each progress tick.
  const rows = useEnvStore((s) => s.rows)
  const selected = useEnvStore((s) => s.selected)
  const phase = useSecurityStore((s) => s.phase)
  const report = useSecurityStore((s) => s.report)
  const reportFor = useSecurityStore((s) => s.reportFor)
  const error = useSecurityStore((s) => s.error)
  const lines = useSecurityStore((s) => s.console)
  const done = useSecurityStore((s) => s.done)
  const total = useSecurityStore((s) => s.total)
  const consoleOpen = useSecurityStore((s) => s.consoleOpen)
  const setConsoleOpen = useSecurityStore((s) => s.setConsoleOpen)
  const run = useSecurityStore((s) => s.run)
  const cancel = useSecurityStore((s) => s.cancel)

  // Where the last save went, so the confirmation names the files rather than merely claiming
  // success. Screen state, not store state: it describes an action, not a report.
  const [saved, setSaved] = useState<string[] | null>(null)

  const row = rows.find((r) => r.interpreter === selected)
  const env = row?.env
  const running = phase === 'running'
  // Only when it describes *this* environment — a plain `report` read would show one
  // environment's advisories after the user switched to another.
  const shown = row === undefined ? null : freshAudit({ report, reportFor }, row.envHash)

  return (
    <div>
      <h1 tabIndex={-1} className="text-accent">
        {t('nav.security')}
      </h1>

      <div className="mt-2 flex flex-wrap items-center gap-2">
        <button
          type="button"
          data-action="run-audit"
          disabled={running || env === undefined}
          onClick={() => {
            if (env !== undefined && row !== undefined) void run(env, row.envHash)
          }}
          className="rounded-pd bg-surface-2 px-3 py-1.5 text-accent disabled:opacity-40"
        >
          {t('security.run')}
        </button>

        {/* Only while a run is going. An audit is 18-68 s — long enough that a user needs a way
            out, which is the measurement that changed P4's answer for Code Health. */}
        {running ? (
          <button
            type="button"
            data-action="cancel-audit"
            onClick={() => {
              void cancel()
            }}
            className="rounded-pd bg-surface-2 px-3 py-1.5 text-text-dim"
          >
            {t('actions.cancel')}
          </button>
        ) : null}

        {shown === null ? null : (
          <button
            type="button"
            onClick={() => {
              void pickSavePath(t('security.saveTitle'), 'security-audit.md').then((target) => {
                if (target !== null) void auditSaveReport(shown, target).then(setSaved)
              })
            }}
            className="rounded-pd bg-surface-2 px-3 py-1.5 text-text"
          >
            {t('security.save')}
          </button>
        )}

        <button
          type="button"
          onClick={() => {
            setConsoleOpen(!consoleOpen)
          }}
          className="ml-auto rounded-pd px-2 py-1 text-data text-text-dim"
        >
          {t('status.log')}
        </button>
      </div>

      {/* One live region for the whole screen. Two would serialize against each other
          unpredictably — the pair P6 folded for exactly this reason. */}
      <p aria-live="polite" className="mt-2 text-data text-text-dim">
        {running ? t('security.running') : saved === null ? '' : t('security.saved', { files: saved.join(', ') })}
      </p>

      {/* A run that failed outright — as opposed to a tool problem inside a report, below. */}
      {error === null ? null : (
        <div className="mt-2">
          <PdErrorRow error={error} />
        </div>
      )}

      {/* pip-audit failed but the report still came back, so the advisories (if any) stay on
          screen beneath this. `⚠ n` counts these rows, per UI-SPEC §3. */}
      {(shown?.problems ?? []).map((problem) => (
        <div key={problem.tool} className="mt-2">
          <PdErrorRow error={asError(problem)} />
        </div>
      ))}

      {/* **Nothing here while a run is going.** `shown` is null then — the report is cleared before
          the command so a second run cannot paint the last one's findings under a new timestamp —
          and rendering the un-run empty state on that basis said *No audit has run for this
          environment* while one was running. That is P4's "no issues found before anything had
          run", inverted: a true-looking sentence derived from a state the screen had not loaded.
          The live region above, the disabled Run and the Cancel beside it already say what is
          happening, so the body says nothing rather than something false. Found by clicking Run
          against a deliberately slow bridge; no test asserted on it, because every test that
          renders this screen renders it settled. */}
      {shown !== null ? (
        <div className="mt-4">
          {shown.cancelled ? (
            // A cancel is a state, not an error, so it is said plainly and the partial result is
            // kept rather than thrown away.
            <p className="text-data text-warn">{t('security.cancelled')}</p>
          ) : null}
          <p className="text-data text-text-dim">
            {t('security.checkedAt', { at: shown.ranAt, version: shown.toolVersion })}
          </p>
          <div className="mt-2">
            <PdAuditReport report={shown} />
          </div>
        </div>
      ) : running ? null : (
        <PdEmptyState
          message={env === undefined ? t('security.noEnv') : t('security.notRunYet')}
          {...(env === undefined ? {} : { hint: t('security.notRunYetHint') })}
        />
      )}

      <PdConsoleDrawer
        lines={lines}
        open={consoleOpen}
        onClose={() => {
          setConsoleOpen(false)
        }}
        done={done}
        total={total}
      />
    </div>
  )
}
