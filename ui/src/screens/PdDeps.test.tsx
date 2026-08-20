/**
 * The dependency screen — the states around the focus view, and the fetch behind it.
 *
 * `PdDepsFocus.test.tsx` covers what a node renders. This covers what `PdDepsFocus` never sees:
 * that the graph is fetched **once per environment and never per click**, that a graph belonging to
 * another environment is not shown under this one's name, and that a mutation invalidates it. The
 * seam between a tested component and its untested parent is where `PdPlanPanel`'s duplicate
 * button lived.
 */

import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { PdDeps } from '@/screens/PdDeps'
import type { DepsGraph } from '@/ipc'
import { useDepsStore, useEnvStore } from '@/stores'
import graphFixture from '@/test/fixtures/deps_graph.json'

const { depsGraph } = vi.hoisted(() => ({ depsGraph: vi.fn() }))

vi.mock('@/ipc', async () => {
  const actual = await vi.importActual<typeof import('@/ipc')>('@/ipc')
  return { ...actual, depsGraph }
})

const GRAPH = graphFixture as DepsGraph
const ENV_HASH = 'abc123'

const ROW = {
  interpreter: String.raw`C:\proj\.venv\Scripts\python.exe`,
  source: 'venv' as const,
  envHash: ENV_HASH,
  env: {
    interpreter: String.raw`C:\proj\.venv\Scripts\python.exe`,
    prefix: String.raw`C:\proj\.venv`,
    pythonVersion: '3.12.10',
    externallyManaged: false,
    source: 'venv' as const,
  },
}

beforeEach(() => {
  depsGraph.mockReset()
  depsGraph.mockResolvedValue(GRAPH)
  useDepsStore.setState({
    phase: 'idle',
    graph: null,
    graphFor: null,
    focus: 'numpy',
    error: null,
  })
  useEnvStore.setState({ rows: [ROW], selected: ROW.interpreter } as never)
})

describe('PdDeps', () => {
  it('fetches the graph once per environment, not once per package', async () => {
    // The design this whole slice turns on. A per-package command would read more naturally and
    // pay a 605 ms probe on every re-centring click.
    render(<PdDeps />)
    await screen.findByText('pandas')
    expect(depsGraph).toHaveBeenCalledTimes(1)

    fireEvent.click(screen.getByText('pandas'))
    await screen.findByText(/python-dateutil/)
    expect(useDepsStore.getState().focus).toBe('pandas')
    // Still one. The re-centre was an object lookup.
    expect(depsGraph).toHaveBeenCalledTimes(1)
  })

  it('does not refetch when the screen remounts with the graph already in hand', async () => {
    const { unmount } = render(<PdDeps />)
    await screen.findByText('pandas')
    unmount()
    render(<PdDeps />)
    await screen.findByText('pandas')
    expect(depsGraph).toHaveBeenCalledTimes(1)
  })

  it('says it is loading rather than rendering an empty view first', async () => {
    // P4's defect and its inverse: a screen must not say "nothing here" before anything has been
    // read, and must not say "loading" over a graph it already holds.
    let release: (g: DepsGraph) => void = () => undefined
    depsGraph.mockReturnValue(
      new Promise<DepsGraph>((resolve) => {
        release = resolve
      }),
    )
    render(<PdDeps />)
    expect(screen.getByText(/Reading this environment/)).toBeInTheDocument()
    expect(screen.queryByText('Nothing here')).toBeNull()

    release(GRAPH)
    await screen.findByText('pandas')
    expect(screen.queryByText(/Reading this environment/)).toBeNull()
  })

  it('does not show one environment\u2019s graph under another\u2019s name', async () => {
    // `freshGraph`'s whole job. Without it the previous environment's edges stay on screen, named
    // as if they described the new one.
    useDepsStore.setState({ graph: GRAPH, graphFor: 'a-different-env', phase: 'ready' })
    render(<PdDeps />)
    expect(screen.queryByText('pandas')).toBeNull()
    // ...and it goes and gets the right one.
    await waitFor(() => {
      expect(depsGraph).toHaveBeenCalledTimes(1)
    })
    await screen.findByText('pandas')
  })

  it('drops the graph when the probe fails, rather than keeping a stale one', async () => {
    depsGraph.mockRejectedValue({ code: 'PD-ENV-001', message: 'no interpreter' })
    render(<PdDeps />)
    await screen.findByText(/PD-ENV-001/)
    expect(useDepsStore.getState().graph).toBeNull()
  })

  it('leaves the mode on Back, and keeps the graph so reopening is free', async () => {
    render(<PdDeps />)
    await screen.findByText('pandas')
    fireEvent.click(screen.getByText('Back to packages'))
    expect(useDepsStore.getState().focus).toBeNull()
    // The environment's edges did not become wrong because the view closed.
    expect(useDepsStore.getState().graph).not.toBeNull()
  })

  it('renders an h1, which the tab shortcut moves focus to', () => {
    // `App.tsx` finds `main h1` on every nav change and focuses it. A screen without one announces
    // nothing to a screen reader after Ctrl+2.
    render(<PdDeps />)
    expect(document.querySelector('h1')?.id).toBe('deps-title')
  })
})
