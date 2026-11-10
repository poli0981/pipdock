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

import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { App } from '@/App'
import { NAV_KEYS } from '@/components/nav'
import type {
  DepsGraph,
  Dist,
  EnvRow,
  ExecutionOutcome,
  ExecutionSummary,
  FlowStep,
  SnapshotMeta,
} from '@/ipc'
import { joinRows } from '@/screens/rows'
import { useDepsStore, useEnvStore, useLegalStore, usePlanStore, useUiStore } from '@/stores'
import graphFixture from '@/test/fixtures/deps_graph.json'
import summaryFixture from '@/test/fixtures/execution_summary.json'
import flowStepForced from '@/test/fixtures/flow_step_forced.json'
import listFixture from '@/test/fixtures/pkg_list.json'
import snapshotFixture from '@/test/fixtures/snapshot_list.json'
import { resetStore } from '@/test/stores'

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
const FORCED_STEP = flowStepForced as FlowStep
const OUTCOME: ExecutionOutcome = {
  summary: summaryFixture as ExecutionSummary,
  snapshot: (snapshotFixture as SnapshotMeta[])[0]!,
}
const ENV_HASH = 'abc123'
const INTERPRETER = String.raw`C:\proj\.venv\Scripts\python.exe`

// Typed, so a seed the app could never produce fails to compile. This one said `source: 'venv'`,
// which is not an `EnvSource`, and lacked `hiddenUserSite`; `as never` below hid both (U10).
const ROW: EnvRow = {
  interpreter: INTERPRETER,
  source: 'venv-scan',
  envHash: ENV_HASH,
  env: {
    interpreter: INTERPRETER,
    prefix: String.raw`C:\proj\.venv`,
    pythonVersion: '3.12.10',
    externallyManaged: false,
    hiddenUserSite: null,
    source: 'venv-scan',
  },
}

beforeEach(() => {
  crossed.length = 0
  // A plan left parked by one test owns the screen in the next, and refuses its shortcuts.
  resetStore(usePlanStore)
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
  legalConsentGet.mockReset().mockResolvedValue({ current: true, docsHash: 'a1b2c3d4e5f60718' })
  useLegalStore.setState({ accepted: true, review: false })
  useUiStore.setState({ nav: 'installed' })
  useDepsStore.setState({
    phase: 'idle',
    graph: null,
    graphFor: null,
    focus: null,
    error: null,
  })
  // The rows `joinRows` builds, not a hand-made shape: the old seed had `pinned`, `state` and a
  // `null` latest, none of which a `PackageRow` has.
  useEnvStore.setState({
    rows: [ROW],
    selected: INTERPRETER,
    dists: DISTS,
    packages: joinRows(DISTS, [], []).rows,
    listing: 'ready',
    outdatedStatus: 'ready',
    selection: new Set<string>(),
  })
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

describe('where focus goes when the screen is replaced', () => {
  // The shell moved focus on a tab change and on nothing else. But four things replace what
  // `<main>` shows without changing the tab — the dependency view, an opened environment, the plan
  // panel, and each of the panel's phases — and every one of them unmounts the control that was
  // just pressed. Focus fell to `<body>`: the next Tab started from the top of the window, and a
  // screen reader announced nothing at all about a screen that had entirely changed.
  const heading = () => screen.getByRole('heading', { level: 1 })

  it('lands on the dependency view when a row opens it', async () => {
    render(<App />)
    const details = within(rowFor('numpy')).getByLabelText('Show dependencies')
    details.focus()
    fireEvent.click(details)
    await screen.findByText('pandas')

    expect(heading()).toHaveTextContent('Dependencies')
    expect(heading()).toHaveFocus()
  })

  it('lands on the table again on the way back', async () => {
    render(<App />)
    fireEvent.click(within(rowFor('numpy')).getByLabelText('Show dependencies'))
    await screen.findByText('pandas')

    const back = screen.getByText('Back to packages')
    back.focus()
    fireEvent.click(back)

    expect(heading()).toHaveTextContent('Installed')
    expect(heading()).toHaveFocus()
  })

  it('lands on the plan panel when a plan takes the screen', async () => {
    render(<App />)
    rowFor('numpy').focus()

    act(() => {
      usePlanStore.setState({ phase: 'preview', kind: 'update', step: FORCED_STEP, env: ROW.env ?? null })
    })

    await waitFor(() => {
      expect(heading()).toHaveFocus()
    })
    expect(heading()).toHaveTextContent('Preview')
  })

  it('lands on the summary when the run ends', async () => {
    render(<App />)
    act(() => {
      usePlanStore.setState({ phase: 'executing', kind: 'update', step: FORCED_STEP, env: ROW.env ?? null })
    })
    const stop = await screen.findByText('Stop')
    stop.focus()

    act(() => {
      usePlanStore.setState({ phase: 'summary', outcome: OUTCOME })
    })

    await waitFor(() => {
      expect(heading()).toHaveFocus()
    })
  })

  it('lands on the first screen when the legal gate is accepted', async () => {
    // Found in the installed build: *Continue* is unmounted with the gate, the shell appears
    // behind it, and the tab has not changed — so the very first screen of a first run had focus
    // on nothing.
    legalConsentGet.mockResolvedValue({ current: false, docsHash: 'a1b2c3d4e5f60718' })
    useLegalStore.setState({ accepted: false, review: false })
    render(<App />)
    const accept = await screen.findByRole('button', { name: 'Continue' })
    accept.focus()

    act(() => {
      useLegalStore.setState({ accepted: true })
    })

    await waitFor(() => {
      expect(heading()).toHaveFocus()
    })
    expect(heading()).toHaveTextContent('Installed')
  })

  it('does not take focus from a control that is still there', async () => {
    // The rule is "focus was lost", not "the phase changed": moving it on every phase would pull
    // a keyboard user off *Console* each time a run advanced.
    render(<App />)
    act(() => {
      usePlanStore.setState({ phase: 'resolving', kind: 'update', env: ROW.env ?? null })
    })
    const back = await screen.findByText('Back')
    back.focus()

    act(() => {
      usePlanStore.setState({ phase: 'preview', step: FORCED_STEP })
    })

    expect(back).toHaveFocus()
  })
})

describe('the keyboard', () => {
  it('selects a tab by the physical digit, whatever the layout types', () => {
    // U49: on AZERTY, Ctrl+3 reports `"`. Parsing `e.key` made every tab shortcut a no-op there.
    render(<App />)
    expect(NAV_KEYS[2]).not.toBe(useUiStore.getState().nav)
    fireEvent.keyDown(window, { key: '"', code: 'Digit3', ctrlKey: true })
    expect(useUiStore.getState().nav).toBe(NAV_KEYS[2])
  })

  it('puts the cursor in the search field when `/` opens Search', async () => {
    // U12: PdSearch focused its field, then the shell's own effect — parents run after children
    // — moved focus to the heading, so the first keystroke after `/` went nowhere.
    render(<App />)
    fireEvent.keyDown(window, { key: '/', code: 'Slash' })
    expect(useUiStore.getState().nav).toBe('search')
    await waitFor(() => {
      expect(screen.getByRole('searchbox')).toHaveFocus()
    })
  })
})
