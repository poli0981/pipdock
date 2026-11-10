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

import { BADGE_TONES as TONES, type BadgeTone } from '@/components/badgeTones'

export type { BadgeTone }

interface PdBadgeProps {
  tone: BadgeTone
  /** Already localized, or data (a version) that must not be translated. */
  label: string
  /**
   * A leading glyph. Text-presentation selectors are worth keeping on emoji here: bare 🔒 renders
   * as a colour emoji from Segoe UI Emoji, which sits badly next to monospace data.
   */
  glyph?: string
  /**
   * What the chip means, when the visible label is a bare glyph or a version.
   *
   * **Rendered as text, not only as a tooltip.** This was a `title` attribute and nothing else,
   * and the comment here called it "the accessible name" — which a `title` on a `<span>` that
   * cannot take focus is not: a pointer could hover it, and a keyboard or a screen reader could
   * not reach it at all. So a held pin read as a bare version number. The sentence is in the
   * document now, visually hidden, and the visible label is hidden from assistive technology in
   * its favour — the sentence already contains the version, and both would say it twice.
   */
  title?: string
}

export function PdBadge({ tone, label, glyph, title }: PdBadgeProps) {
  const explained = title !== undefined && title !== label
  return (
    <span
      // `relative`: the hidden sentence below is `position: absolute`, and with no positioned
      // ancestor it is laid out against the viewport — outside every scroller above it. On Pins a
      // long list of chips made the whole window scroll. The chip is its containing block.
      className={`relative inline-flex shrink-0 items-center gap-1 rounded-pd px-1.5 py-0.5 text-data ${TONES[tone]}`}
      title={title ?? label}
    >
      {glyph === undefined ? null : <span aria-hidden="true">{glyph}</span>}
      <span aria-hidden={explained ? 'true' : undefined}>{label}</span>
      {explained ? <span className="sr-only">{title}</span> : null}
    </span>
  )
}
