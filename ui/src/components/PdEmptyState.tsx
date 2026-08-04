/**
 * An empty state — UI-SPEC §7: "one mono glyph + one sentence + one action".
 *
 * Extracted from the shape `PdEnvironments` had inlined since Stage 1, so both callers render the
 * same thing. The glyph is punctuation, not copy, and is never translated (I18N §2) — which is
 * also why it passes the JSX-literal rule as a template expression rather than bare text.
 */

interface PdEmptyStateProps {
  /** Defaults to the terminal caret the rest of the app uses for a quiet statement of fact. */
  glyph?: string
  /** Already localized. */
  message: string
  /** Optional second line, one shade quieter. */
  hint?: string
  /** Optional single action, per §7. */
  action?: React.ReactNode
}

export function PdEmptyState({ glyph = '▸', message, hint, action }: PdEmptyStateProps) {
  return (
    <div className="mt-8 text-center">
      <p className="font-mono text-text-dim">{`${glyph} ${message}`}</p>
      {hint === undefined ? null : <p className="mt-1 text-data text-text-dim">{hint}</p>}
      {action === undefined ? null : <div className="mt-3">{action}</div>}
    </div>
  )
}
