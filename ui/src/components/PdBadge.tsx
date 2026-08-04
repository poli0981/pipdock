/**
 * A small status chip — UI-SPEC §6.
 *
 * The three the package table needs are `UPDATE` (warn), the pinned lock (info) and, from S3, a
 * danger variant for impossible rows. Tones map onto the design tokens rather than raw colours so
 * the contrast test in `tokens.test.ts` keeps covering them.
 *
 * `label` arrives already localized. A raw string literal here would be an ESLint error
 * (`pipdock/no-jsx-literals`), which is the rule that keeps I18N §1 honest.
 */

export type BadgeTone = 'warn' | 'info' | 'danger' | 'accent' | 'dim'

const TONES: Record<BadgeTone, string> = {
  warn: 'bg-warn/20 text-warn',
  info: 'bg-info/20 text-info',
  danger: 'bg-danger/20 text-danger',
  accent: 'bg-accent/20 text-accent',
  dim: 'border border-border text-text-dim',
}

interface PdBadgeProps {
  tone: BadgeTone
  /** Already localized, or data (a version) that must not be translated. */
  label: string
  /**
   * A leading glyph. Text-presentation selectors are worth keeping on emoji here: bare 🔒 renders
   * as a colour emoji from Segoe UI Emoji, which sits badly next to monospace data.
   */
  glyph?: string
  /** Overrides the accessible name when the visible label is a bare glyph or a version. */
  title?: string
}

export function PdBadge({ tone, label, glyph, title }: PdBadgeProps) {
  return (
    <span
      className={`inline-flex shrink-0 items-center gap-1 rounded-pd px-1.5 py-0.5 text-data ${TONES[tone]}`}
      title={title ?? label}
    >
      {glyph === undefined ? null : <span aria-hidden="true">{glyph}</span>}
      <span>{label}</span>
    </span>
  )
}
