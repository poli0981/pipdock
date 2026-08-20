/**
 * `PdDepsFocus` against the **computed** fixture — PRD P1-6.
 *
 * `deps_graph.json` is produced by `crate::fixtures::deps_graph`, which runs the real
 * `ReverseDeps::view` over the real `pkg_list()`. Hand-writing it would let the graph's rules drift
 * from what this file asserts while the assertions stayed plausible, which is exactly how the
 * held-back preview shipped a doubled sentence from S3 to 1.1.0 with every suite green.
 *
 * No store and no `vi.mock('@/ipc')`: the component takes props. `PdDeps.test.tsx` covers the
 * screen that feeds it — the seam between a tested component and its untested parent is where the
 * `PdPlanPanel` bug lived.
 */

import { fireEvent, render, screen, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { PdDepsFocus } from '@/components/PdDepsFocus'
import type { DepsGraph, DepsNode } from '@/ipc'
import graphFixture from '@/test/fixtures/deps_graph.json'

const GRAPH = graphFixture as DepsGraph
const nodeFor = (pkg: string): DepsNode => {
  const node = GRAPH.nodes[pkg]
  if (node === undefined) throw new Error(`fixture has no node for ${pkg}`)
  return node
}

function setup(pkg: string, overrides: Partial<Parameters<typeof PdDepsFocus>[0]> = {}) {
  const onFocus = vi.fn()
  render(<PdDepsFocus pkg={pkg} node={nodeFor(pkg)} onFocus={onFocus} {...overrides} />)
  return { onFocus }
}

/**
 * A column by its `aria-labelledby`, not by its accessible name.
 *
 * Keying on copy costs a rewrite every time the copy moves, and the first draft of this file
 * proved it: the heading pluralises, so `/requires this/` matched the singular and silently found
 * nothing for the two-dependent case the fixture exists to exercise. The id is the stable thing.
 */
const column = (which: 'dependents' | 'dependencies'): HTMLElement => {
  const el = document.querySelector(`section[aria-labelledby="deps-${which}"]`)
  if (el === null) throw new Error(`no ${which} column rendered`)
  return el as HTMLElement
}
const dependents = () => column('dependents')
const dependencies = () => column('dependencies')
/** A column's heading text, which reports the full count rather than the visible window. */
const heading = (which: 'dependents' | 'dependencies') =>
  document.getElementById(`deps-${which}`)?.textContent

describe('PdDepsFocus', () => {
  it('names both dependents and the specifier each declared', () => {
    // The case both columns exist for: two dependents of `numpy` with two *different* specifiers.
    // A column listing names alone would say these two packages are in the same position, and
    // they are not — one excludes 2.x and the other excludes 1.28+.
    setup('numpy')
    const column = within(dependents())
    expect(column.getByText('pandas')).toBeInTheDocument()
    expect(column.getByText(/<2,>=1\.26\.0/)).toBeInTheDocument()
    expect(column.getByText('scipy')).toBeInTheDocument()
    expect(column.getByText(/<1\.28\.0,>=1\.21\.6/)).toBeInTheDocument()
  })

  it('carries each neighbour version, so a specifier can be checked by hand', () => {
    setup('numpy')
    expect(within(dependents()).getByText(/2\.1\.4/)).toBeInTheDocument()
  })

  it('re-centres on a neighbour rather than fetching', () => {
    // The whole reason `deps_graph` returns one value per environment: this click is a lookup.
    const { onFocus } = setup('numpy')
    fireEvent.click(within(dependents()).getByText('pandas'))
    expect(onFocus).toHaveBeenCalledWith('pandas')
  })

  it('states the transitive counts, which is what the view adds over a single hop', () => {
    setup('numpy')
    expect(screen.getByText(/Removing this would leave 2 packages/)).toBeInTheDocument()
    expect(screen.getByText(/pulls in 0 packages/)).toBeInTheDocument()
  })

  it('reports a requirement nothing satisfies', () => {
    // `pandas` declares `python-dateutil` and the fixture set does not install it. Nothing else in
    // PipDock says so, and it is only trustworthy because Rust evaluated the markers first.
    setup('pandas')
    expect(screen.getByText(/python-dateutil/)).toBeInTheDocument()
  })

  it('says nothing about unsatisfied requirements when there are none', () => {
    // The other direction of P4's defect: a screen that reports an empty list as a finding.
    setup('scipy')
    expect(screen.queryByText(/nothing installed to satisfy/)).toBeNull()
  })

  it('renders a leaf as an honest empty rather than falling through', () => {
    // 32 of the 352-package fixture have no edge in either direction. "Nothing here" twice is the
    // correct answer; a blank panel is the P4 defect wearing a different hat.
    setup('certifi')
    expect(within(dependents()).getByText('Nothing here')).toBeInTheDocument()
    expect(within(dependencies()).getByText('Nothing here')).toBeInTheDocument()
  })

  it('caps a column and counts the overflow from the full set', () => {
    // `setuptools` has 150 dependents in the real 352-package environment. `rowsShown` is
    // overridable so this can be proved on a fixture with two.
    setup('numpy', { rowsShown: 1 })
    const column = within(dependents())
    expect(column.getByText('pandas')).toBeInTheDocument()
    expect(column.queryByText('scipy')).toBeNull()
    expect(column.getByText('1 more not shown')).toBeInTheDocument()
    // ...and the heading still reports the total, not the window.
    expect(heading('dependents')).toBe('2 packages require this')
  })

  it('says a package it has never heard of is not in the listing', () => {
    // A row can be clicked after the package left the environment. An answer, not a crash.
    const onFocus = vi.fn()
    render(<PdDepsFocus pkg="ghost" node={null} onFocus={onFocus} />)
    expect(screen.getByText(/ghost is not in this environment/)).toBeInTheDocument()
  })

  it('states every constraint as text, so forced-colors mode loses nothing', () => {
    // `styles.css` rebinds every --color-* token to a system keyword, collapsing warn, danger and
    // info to CanvasText. A row that leaned on colour to say "this one constrains you" would say
    // nothing at all in that mode, so the specifier is always present as characters.
    setup('numpy')
    for (const edge of nodeFor('numpy').dependents) {
      expect(within(dependents()).getByText(new RegExp(escape(edge.constraint)))).toBeInTheDocument()
    }
  })

  it('spells out an unconstrained edge instead of leaving a gap', () => {
    // An empty specifier means "any version", which is a different statement from "no edge". A
    // blank would read as the second.
    const node: DepsNode = {
      ...nodeFor('numpy'),
      dependents: [{ pkg: 'anything', version: '1.0', constraint: '' }],
    }
    render(<PdDepsFocus pkg="numpy" node={node} onFocus={vi.fn()} />)
    expect(within(dependents()).getByText(/any version/)).toBeInTheDocument()
  })
})

/** Escape a PEP 440 specifier for use inside a RegExp. */
function escape(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}
