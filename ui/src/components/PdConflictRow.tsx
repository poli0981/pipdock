/**
 * One package that needs a decision — UI-SPEC §4's segmented **Keep compatible · Skip · Force
 * latest**, and DATA-FLOW §3's `ConflictDecision` state.
 *
 * Two rules the control has to encode rather than merely display:
 *
 * 1. **`Keep compatible` is disabled on an impossible row.** `plan::default_decision(is_impossible
 *    = true, …)` returns `Skip`, because an impossible package has no compatible version to keep.
 *    Offering a choice the core would refuse to honour is worse than not offering it.
 * 2. **`Force latest` confirms inline, naming what breaks.** DISCLAIMER §2 and UI-SPEC §7 both ask
 *    for it: this is the one control here that knowingly breaks a declared requirement.
 */

import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { PdBadge } from '@/components/PdBadge'
import type { Blocker, Decision } from '@/ipc'

interface PdConflictRowProps {
  pkg: string
  /** Held-back rows have both; an impossible row has neither. */
  resolved?: string
  latest?: string
  /** Why it is stuck. Each is one sentence the user can act on (PRD G2). */
  blockers: readonly Blocker[]
  /** No compatible version exists — the resolver could not satisfy it at all. */
  impossible: boolean
  /** The current answer, or undefined while it is still the default. */
  value: Decision | undefined
  onChoose: (decision: Decision) => void
}

const OPTIONS: readonly Decision[] = ['keep-compatible', 'skip', 'force-latest']

export function PdConflictRow({
  pkg,
  resolved,
  latest,
  blockers,
  impossible,
  value,
  onChoose,
}: PdConflictRowProps) {
  const { t } = useTranslation()
  const [confirmingForce, setConfirmingForce] = useState(false)

  // Mirrors `default_decision`: the safe answer, and the one that costs no extra click in
  // UI-SPEC §5's 4-click budget.
  const effective = value ?? (impossible ? 'skip' : 'keep-compatible')

  return (
    <li
      data-pkg={pkg}
      data-impossible={impossible}
      className={`rounded-pd border-l-2 bg-surface p-3 ${
        impossible ? 'border-danger' : 'border-warn'
      }`}
    >
      <div className="flex flex-wrap items-baseline gap-2">
        <code className="font-mono text-data">{pkg}</code>
        {resolved === undefined ? null : (
          <span className="font-mono text-data text-text-dim">
            {latest === undefined ? resolved : `${resolved} (${latest})`}
          </span>
        )}
        <PdBadge
          tone={impossible ? 'danger' : 'warn'}
          label={impossible ? t('plan.impossible') : t('plan.heldBack')}
        />
      </div>

      {/* One line per blocker, each naming the package and the constraint it declared. Markers
          that do not apply to this interpreter are already filtered out in core (SP-5). */}
      <ul className="mt-1 space-y-0.5">
        {blockers.map((b, i) => (
          <li key={`${b.by ?? ''}-${b.constraint}-${String(i)}`} className="text-data text-text-dim">
            {b.by == null
              ? b.constraint
              : t('plan.blocker', {
                  // Assembled here, from three fields, for the same reason `PdUninstallDialog`
                  // assembles the guard's: a sentence joined in Rust cannot be un-joined by a
                  // catalog. Until Slice 0 `constraint` arrived as a whole English phrase and
                  // this template wrapped it again, naming the dependent twice.
                  by: b.version == null ? b.by : `${b.by} ${b.version}`,
                  constraint: b.constraint,
                })}
          </li>
        ))}
      </ul>

      <div role="radiogroup" aria-label={pkg} className="mt-2 flex gap-1">
        {OPTIONS.map((option) => {
          const disabled = impossible && option === 'keep-compatible'
          const selected = effective === option
          return (
            <button
              key={option}
              type="button"
              role="radio"
              aria-checked={selected}
              disabled={disabled}
              title={disabled ? t('plan.keepCompatibleImpossible') : undefined}
              onClick={() => {
                if (option === 'force-latest') {
                  setConfirmingForce(true)
                  return
                }
                setConfirmingForce(false)
                onChoose(option)
              }}
              className={`rounded-pd border px-2 py-0.5 text-data disabled:opacity-40 ${
                selected ? 'border-accent text-accent' : 'border-border text-text-dim'
              }`}
            >
              {t(`plan.decision.${option}`)}
            </button>
          )
        })}
      </div>

      {confirmingForce ? (
        <div role="alertdialog" aria-label={pkg} className="mt-2 rounded-pd bg-danger/10 p-2">
          <p className="text-data text-danger">
            {t('plan.forceWarning', {
              count: blockers.length,
              names: blockers.map((b) => b.by ?? '?').join(', '),
            })}
          </p>
          <div className="mt-2 flex gap-2">
            {/* Cancel first and focused: UI-SPEC §7 requires the safe option to be the default
                on every destructive confirm. */}
            <button
              type="button"
              autoFocus
              onClick={() => {
                setConfirmingForce(false)
              }}
              className="rounded-pd border border-border px-2 py-0.5 text-data"
            >
              {t('actions.cancel')}
            </button>
            <button
              type="button"
              onClick={() => {
                setConfirmingForce(false)
                onChoose('force-latest')
              }}
              className="rounded-pd bg-danger/20 px-2 py-0.5 text-data text-danger"
            >
              {t('plan.decision.force-latest')}
            </button>
          </div>
        </div>
      ) : null}
    </li>
  )
}
