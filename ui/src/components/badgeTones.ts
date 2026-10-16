/**
 * `PdBadge`'s tones, in a module of their own so `tokens.test.ts` can sweep them.
 *
 * A badge is text on a tint of its own colour, and the tint lowers the contrast the plain token
 * has: danger on a 20 % danger tint over a selected row measured 3.92:1, under WCAG AA's 4.5, and
 * no test looked at it because the token test checked tokens on plain surfaces only (U46). The
 * test now composites each tint here over every surface. Danger takes a lighter tint for that
 * reason, and the token itself moved from #F0574F to #F26A63.
 *
 * Tailwind finds these by scanning source text, so every class is written out whole.
 */

export type BadgeTone = 'warn' | 'info' | 'danger' | 'accent' | 'dim'

export const BADGE_TONES: Readonly<Record<BadgeTone, string>> = {
  warn: 'bg-warn/20 text-warn',
  info: 'bg-info/20 text-info',
  danger: 'bg-danger/15 text-danger',
  accent: 'bg-accent/20 text-accent',
  dim: 'border border-border text-text-dim',
}
