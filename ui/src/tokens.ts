/**
 * The UI-SPEC §2 design tokens, mirrored in TypeScript.
 *
 * `styles.css` is the runtime source of truth (Tailwind's `@theme` generates the utilities); this
 * module exists so the contrast test in `tokens.test.ts` can assert WCAG AA over every text /
 * surface pair without parsing CSS. The two must agree — the test checks that too.
 */

export const colors = {
  bg: '#0A0E0C',
  surface: '#101612',
  surface2: '#161E18',
  border: '#1F2B23',
  accent: '#4ADE80',
  accentDim: '#22935B',
  text: '#E7F0EA',
  textDim: '#8AA394',
  warn: '#F5B94A',
  danger: '#F0574F',
  info: '#5BC0DE',
} as const

export type ColorToken = keyof typeof colors

/** Surfaces text can sit on. */
export const surfaceTokens = ['bg', 'surface', 'surface2'] as const

/** Tokens used for text and iconography. */
export const foregroundTokens = [
  'accent',
  'accentDim',
  'text',
  'textDim',
  'warn',
  'danger',
  'info',
] as const

/** Parse `#RRGGBB` into 0–255 channels. */
export function toRgb(hex: string): [number, number, number] {
  const v = hex.replace('#', '')
  return [
    Number.parseInt(v.slice(0, 2), 16),
    Number.parseInt(v.slice(2, 4), 16),
    Number.parseInt(v.slice(4, 6), 16),
  ]
}

/** Relative luminance per WCAG 2.1. */
export function luminance(hex: string): number {
  const channel = (c: number) => {
    const s = c / 255
    return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4)
  }
  const [r, g, b] = toRgb(hex)
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

/** WCAG contrast ratio between two colours, from 1 to 21. */
export function contrastRatio(a: string, b: string): number {
  const la = luminance(a)
  const lb = luminance(b)
  const [hi, lo] = la > lb ? [la, lb] : [lb, la]
  return (hi + 0.05) / (lo + 0.05)
}

/** WCAG AA for normal-size text. */
export const AA_NORMAL = 4.5

/** WCAG AA for large text (≥18.66px bold or ≥24px). */
export const AA_LARGE = 3
