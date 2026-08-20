/**
 * The shell, driven end to end against a bridge that logs every command — PRD P1-6.
 *
 * This is the jsdom form of the recipe that has caught more real defects in this project than any
 * suite: run the app, stub `invoke`, push every `cmd` into an array, and read back exactly what
 * crossed. Stage 2 found every screen fetching twice on mount that way, and Stage 4 found a plan
 * started from Search resolving into a screen that does not render it — the command ran, the flow
 * parked, and the user saw nothing change. Neither was visible in a passing component suite,
 * because a component test renders one component and these are faults *between* them.
 *
 * The seam under test here is the one no other file covers: `PdPackageRow`'s new details button →
 * `PdPackageTable` → `PdPackages` → `useDepsStore` → `PdDeps` → the bridge. `PdDepsFocus.test.tsx`
 * proves what a node renders and `PdDeps.test.tsx` proves the fetch behaviour; only this proves a
 * user can get there at all, and in how many clicks.
 */

import { fireEvent, render, screen, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { App } from '@/App'
import type { DepsGraph, Dist } from '@/ipc'
import { useDepsStore, useEnvStore, useLegalStore, useUiStore } from '@/stores'
import graphFixture from '@/test/fixtures/deps_graph.json'
import listFixture from '@/test/fixtures/pkg_list.json'

/** Every command that crossed the bridge, in order — the whole point of this file. */
const crossed: string[] = []

const { depsGraph, pkgList, pkgOutdated, pinList, legalConsentGet } = vi.hoisted(() => ({
  depsGraph: vi.fn(),
  pkgList: vi.fn(),
  pkgOutdated: vi.fn(),
  pinList: vi.fn(),
  legalConsentGet: vi.fn(),
}))

vi.mock('@/ipc', async () => {
  const actual = await vi.importActual<typeof import('@/ipc')>('@/ipc')
  return {
    ...actual,
    depsGraph,
    pkgList,
    pkgOutdated,
    pinList,
    // Not optional, and the reason is the same one the browser recipe records: `App` calls
    // `checkConsent()` on mount, and `useLegalStore.check` **fails closed** — an unreadable
    // consent record must never skip the gate. Setting `accepted: true` in the store is undone a
    // tick later by the real command rejecting, and the gate replaces the whole shell. The first
    // draft of this file did exactly that: the row rendered, the click landed, the store went to
    // `phase: 'ready'` with the graph in hand, and every assertion looked at a licence agreement.
    legalConsentGet,
  }
})

const GRAPH = graphFixture as DepsGraph
const DISTS = listFixture as Dist[]
const ENV_HASH = 'abc123'
const INTERPRETER = String.raw`C:\proj\.venv\Scripts\python.exe`

const ROW = {
  interpreter: INTERPRETER,
  source: 'venv' as const,
  envHash: ENV_HASH,
  env: {
    interpreter: INTERPRETER,
    prefix: String.raw`C:\proj\.venv`,
    pythonVersion: '3.12.10',
    externallyManaged: false,
    source: 'venv' as const,
  },
}

beforeEach(() => {
  crossed.length = 0
  // Each mock records its own name on the way in. A generic wrapper reads better and does
  // not typecheck: `ReturnType<typeof vi.fn>` is `Mock<Procedure | Constructable>`, which is
  // not directly callable.
  const record = <T,>(name: string, value: T) => () => {
    crossed.push(name)
    return Promise.resolve(value)
  }
  depsGraph.mockReset().mockImplementation(record('deps_graph', GRAPH))
  pkgList.mockReset().mockImplementation(record('pkg_list', DISTS))
  pkgOutdated.mockReset().mockImplementation(record('pkg_outdated', []))
  pinList.mockReset().mockImplementation(record('pin_list', []))

  // Past the gate, and it takes both halves. The store field is what the first render reads; the
  // command is what the mount effect overwrites it with.
  legalConsentGet.mockReset().mockResolvedValue({ current: true, recorded: true })
  useLegalStore.setState({ accepted: true, review: false })
  useUiStore.setState({ nav: 'installed' })
  useDepsStore.setState({
    phase: 'idle',
    graph: null,
    graphFor: null,
    focus: null,
    error: null,
  })
  useEnvStore.setState({
    rows: [ROW],
    selected: INTERPRETER,
    packages: DISTS.map((d) => ({
      name: d.name,
      version: d.version,
      latest: null,
      sizeBytes: d.sizeBytes ?? null,
      pinned: null,
      state: 'unknown' as const,
    })),
    listing: 'ready',
    outdatedStatus: 'ready',
    selection: new Set<string>(),
  } as never)
})

/** A package row by name, in the virtualized table. */
const rowFor = (name: string) => {
  const row = screen.getByText(name).closest('[role="row"]')
  if (row === null) throw new Error(`no row rendered for ${name}`)
  return row as HTMLElement
}

describe('the dependency view, reached the way a user reaches it', () => {
  it('opens from a package row in one click, and shows that package', async () => {
    // UI-SPEC §5: Installed is already the screen, so this is click 1 of 2. The second is nothing —
    // the view is there. A `⋮` menu would have made it 1 of 3 and pushed uninstall to 4.
    render(<App />)
    fireEvent.click(within(rowFor('numpy')).getByLabelText('Show dependencies'))

    expect(await screen.findByText('pandas')).toBeInTheDocument()
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Dependencies')
    // The focused package, and both its dependents with their differing specifiers.
    expect(screen.getByText(/<2,>=1\.26\.0/)).toBeInTheDocument()
    expect(screen.getByText(/<1\.28\.0,>=1\.21\.6/)).toBeInTheDocument()
  })

  it('crosses the bridge once for the graph, and not again on a re-centre', async () => {
    // The design the whole slice turns on, asserted where it can actually be observed. A
    // `deps_for(pkg)` command would read more naturally and put a 605 ms probe behind this click.
    render(<App />)
    fireEvent.click(within(rowFor('numpy')).getByLabelText('Show dependencies'))
    await screen.findByText('pandas')

    expect(crossed.filter((c) => c === 'deps_graph')).toHaveLength(1)

    fireEvent.click(screen.getByText('pandas'))
    await screen.findByText(/python-dateutil/)
    expect(crossed.filter((c) => c === 'deps_graph')).toHaveLength(1)
  })

  it('does not ask for the graph until someone opens the view', async () => {
    // It costs a probe. `pin_suggestions` is on the Pins screen for this reason and says so; a
    // graph fetched on mount would tax every environment open for a screen most users never open.
    render(<App />)
    await screen.findByText('numpy')
    expect(crossed).not.toContain('deps_graph')
  })

  it('goes back to the table, and the table is still there', async () => {
    render(<App />)
    fireEvent.click(within(rowFor('numpy')).getByLabelText('Show dependencies'))
    await screen.findByText('pandas')

    fireEvent.click(screen.getByText('Back to packages'))
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Installed')
    expect(rowFor('numpy')).toBeInTheDocument()
  })

  it('leaves the row\u2019s other two actions where they were', () => {
    // The reason there are three inline buttons rather than a menu. If this ever regresses to a
    // `⋮`, uninstall goes from three hand-counted clicks to four.
    render(<App />)
    const row = within(rowFor('numpy'))
    expect(row.getByLabelText('Remove this package')).toBeInTheDocument()
    expect(row.getByLabelText('Pin this package')).toBeInTheDocument()
    expect(row.getByLabelText('Show dependencies')).toBeInTheDocument()
    // ...and none of the three is a tab stop; the row is.
    for (const button of row.getAllByRole('button')) {
      expect(button).toHaveAttribute('tabindex', '-1')
    }
  })
})
