import { describe, expect, it } from 'vitest'
import {
  AA_LARGE,
  AA_NORMAL,
  colors,
  contrastRatio,
  foregroundTokens,
  surfaceTokens,
} from './tokens'

import { BADGE_TONES } from '@/components/badgeTones'

/** Tailwind colour names, as `BADGE_TONES` spells them, to the tokens they compile to. */
const TOKEN_OF: Record<string, keyof typeof colors> = {
  warn: 'warn',
  info: 'info',
  danger: 'danger',
  accent: 'accent',
  'text-dim': 'textDim',
}

/** `fg` laid at `alpha` over `bg` — what `bg-danger/15` composites to on a surface. */
function tint(fg: string, bg: string, alpha: number): string {
  const channel = (i: number) =>
    Math.round(
      Number.parseInt(fg.slice(1 + 2 * i, 3 + 2 * i), 16) * alpha +
        Number.parseInt(bg.slice(1 + 2 * i, 3 + 2 * i), 16) * (1 - alpha),
    )
      .toString(16)
      .padStart(2, '0')
  return `#${channel(0)}${channel(1)}${channel(2)}`
}

/**
 * UI-SPEC §8: "all text tokens meet WCAG AA on their surfaces (verified in the design-token
 * test)". This is that test — TESTING L3 lists it as a build-time gate, so a token tweak that
 * quietly drops contrast fails CI rather than shipping.
 */
describe('design tokens', () => {
  it.each(surfaceTokens)('primary text meets AA on %s', (surface) => {
    expect(contrastRatio(colors.text, colors[surface])).toBeGreaterThanOrEqual(AA_NORMAL)
  })

  it.each(surfaceTokens)('dimmed text meets AA on %s', (surface) => {
    // The "dimmed up-to-date rows" requirement (UI-SPEC §4) must stay readable, not decorative.
    expect(contrastRatio(colors.textDim, colors[surface])).toBeGreaterThanOrEqual(AA_NORMAL)
  })

  it.each(['accent', 'warn', 'danger', 'info'] as const)(
    '%s meets AA on the app background',
    (token) => {
      expect(contrastRatio(colors[token], colors.bg)).toBeGreaterThanOrEqual(AA_NORMAL)
    },
  )

  it('accent-dim clears AA on the app background', () => {
    // Secondary accents and links per UI-SPEC §2. It clears AA_NORMAL on --color-bg (4.99), so
    // it is safe for body-size links there — but see the large-text sweep below for the darker
    // ratio it gets on elevated surfaces.
    expect(contrastRatio(colors.accentDim, colors.bg)).toBeGreaterThanOrEqual(AA_NORMAL)
  })

  it.each(Object.entries(BADGE_TONES))('the %s badge reads at AA on every surface', (tone, classes) => {
    // U46: the token test checked tokens on plain surfaces, and a badge is its text on a tint of
    // its own colour — danger read 3.92:1 on a selected row with nothing to say so.
    const text = TOKEN_OF[/text-([a-z-]+)/.exec(classes)?.[1] ?? '']
    expect(text, `${tone} names a text token`).toBeDefined()
    const fill = /bg-([a-z-]+)\/(\d+)/.exec(classes)
    for (const surface of surfaceTokens) {
      const under =
        fill === null
          ? colors[surface]
          : tint(colors[TOKEN_OF[fill[1] ?? ''] ?? 'bg'], colors[surface], Number(fill[2]) / 100)
      expect(
        contrastRatio(colors[text ?? 'text'], under),
        `${tone} on ${surface}`,
      ).toBeGreaterThanOrEqual(AA_NORMAL)
    }
  })

  it('every foreground token clears AA large on every surface', () => {
    for (const fg of foregroundTokens) {
      for (const surface of surfaceTokens) {
        expect(
          contrastRatio(colors[fg], colors[surface]),
          `${fg} on ${surface}`,
        ).toBeGreaterThanOrEqual(AA_LARGE)
      }
    }
  })
})
