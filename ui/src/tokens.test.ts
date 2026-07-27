import { describe, expect, it } from 'vitest'
import {
  AA_LARGE,
  AA_NORMAL,
  colors,
  contrastRatio,
  foregroundTokens,
  surfaceTokens,
} from './tokens'

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
