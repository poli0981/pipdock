/**
 * Every destructive control says so in an attribute, and high contrast draws it — UI-SPEC §7.
 *
 * Forced-colors drops every tint, so a danger-treated primary is drawn exactly like the Cancel
 * beside it unless something that survives the mode marks it. The stylesheet used to do that with
 * a hand-kept list of five `data-action` values, and the list was wrong for as long as it existed:
 * *Remove*, *Allow*, *Clear*, the *Force latest* confirm and a forced plan's *Confirm* were never
 * on it (audit backlog U45). A list of names beside a list of buttons is two lists.
 *
 * So the mark is one attribute, `data-destructive`, and this reads the source to hold both halves:
 * every button drawn in the danger colour carries it, and the stylesheet draws it. jsdom applies
 * no stylesheet and has no forced-colors, so there is nothing to render — the source is the only
 * place either fact exists before the hand pass.
 */

import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

import { describe, expect, it } from 'vitest'

const sources = import.meta.glob<string>(['./**/*.tsx', '!./**/*.test.tsx'], {
  query: '?raw',
  import: 'default',
  eager: true,
})
// Read from disk: vitest hands a test an empty string for any `.css` import, `?raw` included.
// From the working directory, which `vite.config.ts` pins to the repository root — under jsdom
// `import.meta.url` is not a `file:` URL to resolve against.
const styles = readFileSync(resolve(process.cwd(), 'ui/src/styles.css'), 'utf8')

/** The opening tag of every `<button>` in a file, as written. */
function buttonTags(source: string): string[] {
  const lines = source.split('\n')
  const tags: string[] = []
  for (let i = 0; i < lines.length; i += 1) {
    if (!/<button\b/.test(lines[i] ?? '')) continue
    const tag: string[] = []
    // The formatter closes a multi-line opening tag with `>` on a line of its own, which is the
    // one `>` in a JSX tag that cannot be an arrow function or a comparison.
    for (let j = i; j < lines.length; j += 1) {
      const line = lines[j] ?? ''
      tag.push(line)
      if (line.trim() === '>' || (j === i && /<button\b[^{]*>/.test(line))) break
    }
    tags.push(tag.join('\n'))
  }
  return tags
}

/** Actions that are destructive whatever colour their button is drawn in. */
const DESTRUCTIVE_ACTIONS =
  /data-action="(force|uninstall|rollback|upgrade-pip|fix|remove|allow-override|confirm-clear-cache)"/

describe('destructive controls', () => {
  const tags = Object.entries(sources).flatMap(([file, source]) =>
    buttonTags(source).map((tag) => ({ file, tag })),
  )

  it('finds the buttons it is meant to be reading', () => {
    // A scan that matches nothing passes everything.
    expect(tags.length).toBeGreaterThan(40)
    expect(tags.filter(({ tag }) => /(border|bg)-danger/.test(tag)).length).toBeGreaterThan(8)
  })

  it('marks every button drawn in the danger colour', () => {
    const unmarked = tags
      .filter(({ tag }) => /(border|bg)-danger/.test(tag) && !tag.includes('data-destructive'))
      .map(({ file, tag }) => `${file}: ${tag.trim().split('\n').slice(0, 3).join(' ')}`)
    expect(unmarked).toEqual([])
  })

  it('marks every action that removes, forces, rolls back or rewrites', () => {
    const unmarked = tags
      .filter(({ tag }) => DESTRUCTIVE_ACTIONS.test(tag) && !tag.includes('data-destructive'))
      .map(({ file, tag }) => `${file}: ${tag.trim().split('\n').slice(0, 3).join(' ')}`)
    expect(unmarked).toEqual([])
  })

  it('is drawn by the stylesheet under forced-colors, by the attribute and not by a list', () => {
    const forced = styles.slice(styles.indexOf('@media (forced-colors: active)'))
    expect(forced).toContain('button[data-destructive]')
    expect(forced).not.toMatch(/button\[data-action=/)
  })

  it('outlines the sidebar tab that is showing', () => {
    // The tab carries `aria-current="page"`; the rule matched `'true'`, so in high contrast the
    // active tab looked like every other.
    const forced = styles.slice(styles.indexOf('@media (forced-colors: active)'))
    expect(forced).toContain("[aria-current='page']")
  })
})
