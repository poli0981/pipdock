/**
 * axe-core against a rendered tree — TESTING §2's accessibility tier.
 *
 * axe is called directly rather than through a matcher package. The matcher packages are thin
 * wrappers pinned to a test runner's major version, and `eslint-plugin-jsx-a11y` — the other usual
 * gate — peers ESLint 9 at most while this repository is on 10. axe-core itself has no
 * dependencies and no opinion about the runner.
 *
 * **What this cannot see, so that nobody reads a pass as more than it is:**
 *
 * - `color-contrast` is off. jsdom computes no layout and no real colours, so the rule would
 *   report "incomplete" for every node. Contrast is `tokens.test.ts`'s job, which computes it from
 *   the tokens themselves.
 * - Nothing here knows what a screen reader *says*, what forced-colors draws, or where focus goes
 *   after a keypress. Those are the hand tier and the focus tests.
 *
 * It returns strings rather than asserting, so the failure message is the list of what is wrong
 * and where — `expect(await violations()).toEqual([])` prints every rule and selector.
 */

import axe from 'axe-core'

interface Options {
  /**
   * `region` asks that all content sit inside a landmark. True of the whole app, and meaningless
   * for a component rendered alone into a bare `<div>`.
   */
  page?: boolean
}

export async function violations(
  root: Element = document.body,
  { page = false }: Options = {},
): Promise<string[]> {
  const results = await axe.run(root, {
    rules: {
      'color-contrast': { enabled: false },
      region: { enabled: page },
    },
  })
  return results.violations.flatMap((violation) =>
    violation.nodes.map((node) => `${violation.id}: ${violation.help} — ${node.target.join(' ')}`),
  )
}
