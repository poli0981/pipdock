/**
 * A finished Code Health run — UI-SPEC §6's fifteenth component, CODE-HEALTH-SPEC §5's tabs.
 *
 * Prop-driven and store-free, the way `PdPreviewDiff` and `PdRollbackPreview` are split from the
 * screens that own their data. That split is what lets this be tested against a committed fixture
 * with nothing mocked; `PdHealth` owns the folder, the run, the progress and the errors.
 *
 * **Everything the tools said is rendered verbatim.** deptry codes, ruff rule codes and names,
 * vulture messages, file paths, versions — all data, none of it translated (I18N §2). Only the
 * furniture between them comes from `t()`. In particular there is no PipDock copy per `DEP` code:
 * deptry adds codes on its own schedule and the mapping would be maintained in two languages
 * against a tool this project does not control.
 */

import { openUrl } from '@tauri-apps/plugin-opener'
import { useTranslation } from 'react-i18next'

import { PdBadge } from '@/components/PdBadge'
import { PdEmptyState } from '@/components/PdEmptyState'
import { HEALTH_TABS, tabState, type HealthTab, type RuffFileGroup } from '@/stores'
import type { DeptryIssue, HealthReport, RuffFinding, VultureFinding } from '@/ipc'

/**
 * How many ruff findings to render before asking.
 *
 * ruff over a large tree routinely emits thousands, and its built-in defaults are much wider than
 * the classic `E4,E7,E9,F` — with no config at all it still reports flake8-bandit rules. Bounded
 * rather than virtualized: `PdPackageTable` is a fixed-column ARIA grid typed to `PackageRow`, and
 * generalizing it over three row shapes is a bigger change than the problem. The **tab counts
 * always come from the report**, so a capped list never misreports a total.
 */
export const RUFF_ROWS_SHOWN = 500

/** vulture's own guidance on suppressing a false positive (CODE-HEALTH-SPEC §6). */
const VULTURE_WHITELIST = 'https://github.com/jendrikseipp/vulture#handling-false-positives'

interface PdHealthReportProps {
  report: HealthReport
  /** ruff grouped by file, from the store. Never regrouped here — see `groupRuff`. */
  ruffByFile: RuffFileGroup[]
  tab: HealthTab
  onTab: (tab: HealthTab) => void
  /**
   * Installed distribution names, PEP 503-normalized, for the deptry handoff.
   *
   * Empty until `pkg_list` resolves, which is why the button is rendered from a *match* rather
   * than from the absence of one: an unknown name gets no button, never a broken one.
   */
  installed: readonly string[]
  /** Hand a distribution to the uninstall guard. Absent means the handoff is not wired yet. */
  onUninstall?: ((distribution: string) => void) | undefined
  /** Open the fix confirm. Absent means the write path is not available to this caller. */
  onFix?: (() => void) | undefined
  /** How many ruff findings to show; overridable so a test does not have to build 500 rows. */
  rowsShown?: number
}

/** How many findings each tab is reporting. Read off the report, never recomputed. */
function counts(report: HealthReport): Record<HealthTab, number> {
  return {
    deptry: report.deptry.length,
    vulture: report.vulture.length,
    ruff: report.ruff.findings.length,
  }
}

/**
 * `file`, or `file:line` when the tool knew one. Always mono, always verbatim.
 *
 * `line` takes `undefined` as well as `null` because `exactOptionalPropertyTypes` is on and the
 * generator emits `?: T | null` for every `Option<T>` — so an absent field and a null one are
 * different types and both reach here. deptry produces the absent case for a finding about the
 * manifest, which has no line to name.
 */
function Where({ file, line }: { file: string; line?: number | null | undefined }) {
  return <code className="font-mono text-data text-text-dim">{line == null ? file : `${file}:${line}`}</code>
}

function DeptryRow({
  issue,
  installed,
  onUninstall,
}: {
  issue: DeptryIssue
  installed: readonly string[]
  onUninstall?: ((distribution: string) => void) | undefined
}) {
  const { t } = useTranslation()
  // PEP 503 on both sides. `Dist.name` already arrives normalized from `PkgName::parse`; deptry's
  // does not, because it is a module name and was never a distribution name to begin with.
  const normalized = issue.dep.toLowerCase().replace(/[-_.]+/g, '-')
  const match = installed.includes(normalized) ? normalized : null
  // **Two gates, and the first is the one that matters.** Only DEP002 — an *unused* dependency —
  // can be uninstalled. DEP001 is missing, so offering to remove it would tell the user to
  // uninstall something the project does not have; DEP003 is transitive and removing it is
  // actively wrong. CODE-HEALTH-SPEC §5 says "unused-dep rows", and the qualifier is load-bearing.
  const offerUninstall = issue.code === 'DEP002' && match !== null && onUninstall !== undefined

  return (
    <li className="flex flex-wrap items-baseline gap-2 border-b border-border py-1.5">
      <PdBadge tone="warn" label={issue.code} />
      <code className="font-mono text-data text-text">{issue.dep}</code>
      <span className="min-w-0 flex-1 text-data text-text-dim">{issue.message}</span>
      {offerUninstall ? (
        <button
          type="button"
          className="shrink-0 rounded-pd border border-border px-2 py-0.5 text-data text-accent-dim hover:bg-surface-2"
          onClick={() => {
            onUninstall(match)
          }}
        >
          {t('health.reviewInUninstall')}
        </button>
      ) : null}
      {(issue.locations ?? []).length > 0 ? (
        <ul className="w-full pl-6">
          {(issue.locations ?? []).map((loc) => (
            <li key={`${loc.file}:${loc.line ?? ''}`}>
              <Where file={loc.file} line={loc.line} />
            </li>
          ))}
        </ul>
      ) : null}
    </li>
  )
}

function VultureRow({ finding }: { finding: VultureFinding }) {
  return (
    <li className="flex flex-wrap items-baseline gap-2 border-b border-border py-1.5">
      <Where file={finding.path} line={finding.line} />
      <span className="min-w-0 flex-1 text-data text-text">{finding.message}</span>
      {/* 100% is a fact; below it is a candidate, which §6 says to say rather than imply. */}
      <PdBadge tone={finding.confidence === 100 ? 'warn' : 'dim'} label={`${finding.confidence}%`} />
    </li>
  )
}

function RuffRow({ finding }: { finding: RuffFinding }) {
  const { t } = useTranslation()
  // `code` is optional because the field is not ours; a syntax error carries `invalid-syntax`
  // rather than null, so this fallback is format insurance, not a branch real input reaches.
  const code = finding.code ?? finding.name
  return (
    <li className="flex flex-wrap items-baseline gap-2 border-b border-border py-1.5">
      <code className="font-mono text-data text-text-dim">{`${finding.row}:${finding.column}`}</code>
      {finding.url == null ? (
        <code className="font-mono text-data text-text">{code}</code>
      ) : (
        <button
          type="button"
          className="font-mono text-data text-accent-dim underline-offset-2 hover:underline"
          onClick={() => {
            // Fire-and-forget, exactly as `PdErrorRow` opens its links. The URL is ruff's own —
            // constructed ones 404, because the rule page is keyed by name and not by code.
            void openUrl(finding.url ?? '')
          }}
        >
          {code}
        </button>
      )}
      <span className="min-w-0 flex-1 text-data text-text">{finding.message}</span>
      {finding.fix === 'safe' ? <PdBadge tone="accent" label={t('health.fixableBadge')} /> : null}
      {finding.fix === 'unsafe' ? <PdBadge tone="dim" label={t('health.unsafeBadge')} /> : null}
    </li>
  )
}

export function PdHealthReport({
  report,
  ruffByFile,
  tab,
  onTab,
  installed,
  onUninstall,
  onFix,
  rowsShown = RUFF_ROWS_SHOWN,
}: PdHealthReportProps) {
  const { t } = useTranslation()
  const found = counts(report)
  const state = tabState(report, tab, found[tab])

  // Rendered once, above whichever tab is showing, rather than per tab: the same finding count
  // reached from three places would be three chances to disagree.
  const declared =
    report.declared.kind === 'requirements'
      ? t('health.declared.requirements', { files: report.declared.files.join(', ') })
      : t(`health.declared.${report.declared.kind}`)

  let shown = 0
  const capped: RuffFileGroup[] = []
  for (const group of ruffByFile) {
    if (shown >= rowsShown) break
    const room = rowsShown - shown
    capped.push(
      group.findings.length <= room ? group : { ...group, findings: group.findings.slice(0, room) },
    )
    shown += Math.min(group.findings.length, room)
  }
  const remaining = found.ruff - shown

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <p className="shrink-0 text-data text-text-dim">{declared}</p>

      <div role="tablist" className="mt-3 flex shrink-0 gap-1 border-b border-border">
        {HEALTH_TABS.map((key) => (
          <button
            key={key}
            role="tab"
            type="button"
            aria-selected={tab === key}
            // The visible label is the tool's own name — data, never translated (I18N §2). The
            // accessible name is what says which question the tab answers, and the count with it.
            aria-label={t(`health.tabLabel.${key}`, { count: found[key] })}
            className={`rounded-t-pd px-3 py-1.5 font-mono text-data ${
              tab === key
                ? 'border-b-2 border-accent text-text'
                : 'text-text-dim hover:bg-surface-2'
            }`}
            onClick={() => {
              onTab(key)
            }}
          >
            {key}
            {/* The count comes off the report. A tab that recomputed it could disagree with the
                CLI over the same run, which is the one difference nobody would think to check. */}
            <span className="ml-2 text-text-dim">{found[key]}</span>
          </button>
        ))}
      </div>

      <div className="mt-3 min-h-0 flex-1 overflow-auto">
        {state === 'notRun' ? (
          <PdEmptyState message={t('health.notRun')} hint={t('health.notRunHint')} />
        ) : null}
        {state === 'failed' ? (
          <PdEmptyState message={t('health.failed')} hint={t('health.failedHint')} />
        ) : null}
        {state === 'clean' ? <PdEmptyState message={t('health.clean')} /> : null}

        {state === 'findings' && tab === 'deptry' ? (
          <>
            {/* The screen half of CODE-HEALTH-SPEC §3's amendment: the CLI prints this beneath
                its findings under the same condition, and the two heads must not disagree about
                when the caveat applies. */}
            <p className="mb-2 border-l-2 border-info pl-2 text-data text-info">
              {t('health.deptryNote')}
            </p>
            <ul>
              {report.deptry.map((issue) => (
                <DeptryRow
                  key={`${issue.code}:${issue.dep}`}
                  issue={issue}
                  installed={installed}
                  onUninstall={onUninstall}
                />
              ))}
            </ul>
          </>
        ) : null}

        {state === 'findings' && tab === 'vulture' ? (
          <>
            <p className="mb-2 text-data text-text-dim">
              {t('health.vultureHint')}{' '}
              <button
                type="button"
                className="text-accent-dim underline-offset-2 hover:underline"
                onClick={() => {
                  void openUrl(VULTURE_WHITELIST)
                }}
              >
                {t('health.vultureWhitelist')}
              </button>
            </p>
            <ul>
              {report.vulture.map((finding) => (
                <VultureRow key={`${finding.path}:${finding.line}:${finding.message}`} finding={finding} />
              ))}
            </ul>
          </>
        ) : null}

        {state === 'findings' && tab === 'ruff' ? (
          <>
            {/* The count is `fixable` off the report, never the badges rendered below: the button,
                the CLI prompt and the server's own check all have to name one number. */}
            {onFix !== undefined && report.ruff.fixable > 0 ? (
              <button
                type="button"
                data-action="fix"
                className="mb-3 rounded-pd bg-danger px-3 py-1 text-data text-bg"
                onClick={onFix}
              >
                {t('health.fix', { count: report.ruff.fixable })}
              </button>
            ) : null}
            {capped.map((group) => (
              // Sectioned groups, as `PdPreviewDiff` groups its changes: a flat list of a
              // thousand lint findings answers no question anyone actually has.
              <section key={group.file} aria-labelledby={`ruff-${group.file}`} className="mb-4">
                <h2
                  id={`ruff-${group.file}`}
                  className="flex items-baseline gap-2 font-mono text-data text-text-dim"
                >
                  {group.file}
                  <span>{t('health.ruffFileCount', { count: group.findings.length })}</span>
                </h2>
                <ul>
                  {group.findings.map((finding) => (
                    <RuffRow
                      key={`${finding.filename}:${finding.row}:${finding.column}:${finding.name}`}
                      finding={finding}
                    />
                  ))}
                </ul>
              </section>
            ))}
            {remaining > 0 ? (
              <p className="text-data text-text-dim">
                {t('health.showRemaining', { count: remaining })}
              </p>
            ) : null}
          </>
        ) : null}
      </div>

      {/* CODE-HEALTH-SPEC §7 asks for this in the report footer. Rendered as a constant rather
          than carried on the wire: the exclusion is compiled into the argv and unconditional, so
          a field would transmit a build-time `true` at the cost of a schema change. */}
      <p className="mt-2 shrink-0 text-data text-text-dim">{t('health.notebooks')}</p>
    </div>
  )
}
