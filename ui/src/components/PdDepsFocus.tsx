/**
 * The dependency focus view — PRD P1-6's "who holds this back", UI-SPEC §4.
 *
 * Three columns: what requires the focused package, the package, and what it requires. Clicking a
 * neighbour re-centres, so **depth is navigated rather than drawn**.
 *
 * # Why depth 1, and why this is not a node-link diagram
 *
 * Measured against the 352-package fixture before any of this was designed. A depth-1
 * neighbourhood is a **median of 7 nodes** and a p90 of 25 — small enough to read at a glance.
 * Depth 2 is a **median of 172** and exceeds 60 nodes for **212 of 352 packages**. No layout
 * algorithm makes 172 nodes legible, so a graph library would have bought an eleventh runtime
 * dependency, a `THIRD-PARTY-NOTICES.md` edit that re-shows the legal gate, and a CSP argument —
 * `tauri.conf.json` sets no `script-src`, so anything reaching for `eval` is refused — in exchange
 * for a picture nobody can use.
 *
 * The transitive question is answered by **numbers**: `impact` and `reach` come from Rust's own
 * closures. Paths are not offered at all, because there are more than 2000 distinct paths from
 * `setuptools` to a root in that same environment; "why is this here" has no path-shaped answer at
 * this scale.
 *
 * # Rendering rules this had to obey
 *
 * This is the first visual primitive in the app, and `styles.css`'s `forced-colors` block rebinds
 * every `--color-*` token to a system keyword — `warn`, `danger` and `info` all collapse to
 * `CanvasText`. So **nothing here is carried by colour alone**: every edge states its constraint
 * as text, and the two columns are told apart by their headings and their position, not their
 * hue. The connectors are borders rather than SVG strokes, which keeps them visible in that mode
 * and needs no `viewBox` to stay right at the 1600×1100 window ceiling.
 *
 * Prop-driven and store-free, like `PdAuditReport`: it can be tested against a fixture with no
 * store at all, and `rowsShown` is overridable so the cap can be exercised without 150 rows.
 */

import { useTranslation } from 'react-i18next'

import type { DepEdge, DepsNode } from '@/ipc'

/**
 * Rows per column before the "+ N more" line.
 *
 * `SUGGESTIONS_SHOWN`'s rule, for its reason: `setuptools` has **150** dependents in the
 * 352-package fixture, and a column of 150 is not a view, it is a second package list. The count
 * in the overflow line comes from the **full** array, so a capped column never misreports a total.
 */
export const DEPS_ROWS_SHOWN = 8

interface PdDepsFocusProps {
  /** The focused package's name. */
  pkg: string
  /** Its node, or null when the graph has never heard of it. */
  node: DepsNode | null
  /** Re-centre on a neighbour. */
  onFocus: (pkg: string) => void
  /** Overridable so a test can exercise the cap without a 150-row fixture. */
  rowsShown?: number
}

function Column({
  labelId,
  heading,
  edges,
  rowsShown,
  onFocus,
  side,
}: {
  labelId: string
  heading: string
  edges: DepEdge[]
  rowsShown: number
  onFocus: (pkg: string) => void
  /** Which way the connector points. Decoration only — the heading carries the meaning. */
  side: 'left' | 'right'
}) {
  const { t } = useTranslation()
  const shown = edges.slice(0, rowsShown)
  const hidden = edges.length - shown.length

  return (
    <section aria-labelledby={labelId} className="min-w-0 flex-1">
      <h2 id={labelId} className="text-data text-text-dim">
        {heading}
      </h2>
      <ul className="mt-2 flex flex-col gap-1">
        {shown.map((edge) => (
          <li key={edge.pkg} className="flex min-w-0 items-center gap-2">
            {side === 'right' ? (
              <span aria-hidden="true" className="shrink-0 text-text-dim">
                {'─'}
              </span>
            ) : null}
            <button
              type="button"
              onClick={() => {
                onFocus(edge.pkg)
              }}
              title={t('deps.focusOn', { pkg: edge.pkg })}
              className="min-w-0 flex-1 truncate rounded-pd border border-border px-2 py-0.5 text-left font-mono text-data"
            >
              <span className="text-text">{edge.pkg}</span>
              {edge.version === null || edge.version === undefined ? null : (
                <span className="text-text-dim">{` ${edge.version}`}</span>
              )}
              {/* The specifier, always as text. In forced-colors every token above collapses to
                  CanvasText, so a row that leaned on colour to say "this one constrains you"
                  would say nothing at all. */}
              <span className="text-text-dim">
                {edge.constraint === ''
                  ? ` ${t('deps.unconstrained')}`
                  : ` ${edge.constraint}`}
              </span>
            </button>
            {side === 'left' ? (
              <span aria-hidden="true" className="shrink-0 text-text-dim">
                {'─'}
              </span>
            ) : null}
          </li>
        ))}
      </ul>
      {hidden > 0 ? (
        <p className="mt-2 text-data text-text-dim">{t('deps.more', { count: hidden })}</p>
      ) : null}
      {edges.length === 0 ? (
        <p className="mt-2 text-data text-text-dim">{t('deps.none')}</p>
      ) : null}
    </section>
  )
}

export function PdDepsFocus({
  pkg,
  node,
  onFocus,
  rowsShown = DEPS_ROWS_SHOWN,
}: PdDepsFocusProps) {
  const { t } = useTranslation()

  // A package can leave the environment between the fetch and the click that focuses it. Saying
  // so is a real answer; throwing would turn a stale row into a crash.
  if (node === null) {
    return (
      <p className="text-data text-text-dim">{t('deps.unknownPackage', { pkg })}</p>
    )
  }

  return (
    <div className="flex flex-col gap-4">
      {/* Stacks below `md`. The window tops out at 1600×1100 and cannot be maximized, so this is
          not a phone breakpoint — it is the narrow end of a real window, where three columns of
          package names would each be too narrow to read a version in. */}
      <div className="flex flex-col items-stretch gap-4 md:flex-row md:items-center">
        <Column
          labelId="deps-dependents"
          heading={t('deps.dependents', { count: node.dependents.length })}
          edges={node.dependents}
          rowsShown={rowsShown}
          onFocus={onFocus}
          side="left"
        />

        <div className="shrink-0 rounded-pd border border-accent px-4 py-2 text-center">
          <p className="font-mono text-data text-accent">{pkg}</p>
          {node.version === null || node.version === undefined ? null : (
            <p className="font-mono text-data text-text-dim">{node.version}</p>
          )}
        </div>

        <Column
          labelId="deps-dependencies"
          heading={t('deps.dependencies', { count: node.dependencies.length })}
          edges={node.dependencies}
          rowsShown={rowsShown}
          onFocus={onFocus}
          side="right"
        />
      </div>

      {/* The transitive answer, and the only thing this view adds over what the app already did
          single-hop. `impact` is `removal_closure` without the package itself; `reach` is its
          forward mirror. Both are counts rather than a drawing, for the reason in the module doc. */}
      <p className="text-data text-text-dim">
        <span>{t('deps.impact', { count: node.impact })}</span>
        <span>{' '}</span>
        <span>{t('deps.reach', { count: node.reach })}</span>
      </p>

      {node.unsatisfied !== undefined && node.unsatisfied.length > 0 ? (
        <p className="text-data text-warn">
          {t('deps.unsatisfied', {
            count: node.unsatisfied.length,
            packages: node.unsatisfied.join(', '),
          })}
        </p>
      ) : null}
    </div>
  )
}
