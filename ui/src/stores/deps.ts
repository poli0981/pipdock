/**
 * The dependency view — PRD P1-6, UI-SPEC §4.
 *
 * Its own file, re-exported from `@/stores`, for the reason `health.ts` and `security.ts` are: a
 * domain that owns a screen owns a file.
 *
 * **One fetch per environment, not one per package.** `graph` holds every installed package's
 * edges at once, so re-centring the view is an object lookup rather than a round trip. That is the
 * whole reason `deps_graph` is shaped the way it is — a per-package command would pay a 605 ms
 * probe on every click.
 *
 * Nothing here parses a requirement or evaluates a marker. Which edges are in force, which are
 * gated behind an extra, and which requirements nothing satisfies are all decided in Rust and
 * arrive decided. A second implementation on this side would drift from the uninstall guard, and
 * two features disagreeing about one edge is the failure `graph/mod.rs` exists to prevent.
 */

import { create } from 'zustand'

import { depsGraph, isPdError, type DepsGraph, type DepsNode, type PdError, type PyEnv } from '@/ipc'

/** Where a fetch is. */
export type DepsPhase = 'idle' | 'loading' | 'ready' | 'failed'

export interface DepsState {
  phase: DepsPhase
  /** Every installed package's edges, or null when nothing has been fetched. */
  graph: DepsGraph | null
  /**
   * Which environment `graph` describes.
   *
   * Held separately so one environment's edges can never be shown under another's name — the
   * reason `useSecurityStore` keeps `reportFor` and `useHealthStore` keeps `reportFor`.
   */
  graphFor: string | null
  /**
   * The package the view is centred on, and the thing that makes this a *mode*.
   *
   * `null` means the package table is showing. `PdPackages` switches on it exactly as
   * `PdEnvironments` switches on `openFor` — UI-SPEC §4 puts Snapshots under Environments for the
   * same reason a dependency view belongs under Installed: `Ctrl+1..9` is positional over
   * `NAV_KEYS`, so a tenth tab beside Installed would be an *insert* and would rebind every
   * shortcut after it.
   */
  focus: string | null
  error: PdError | null
  /** Enter the view centred on `pkg`. */
  open: (pkg: string) => void
  /** Re-centre without leaving. */
  refocus: (pkg: string) => void
  /** Back to the package table. */
  close: () => void
  load: (env: PyEnv, envHash: string) => Promise<void>
  reset: () => void
}

/**
 * Cleared when the selected environment changes.
 *
 * `focus` is inside it, unlike `security.ts`'s `consoleOpen`: which package you were looking at is
 * a fact about an environment, not a preference about the screen, and a name carried across a
 * switch would centre the view on a package the new environment may not have.
 */
const NO_DEPS = {
  phase: 'idle' as DepsPhase,
  graph: null,
  graphFor: null,
  focus: null,
  error: null,
}

const asPdError = (e: unknown): PdError =>
  isPdError(e) ? e : { code: 'PD-INT-001', message: String(e) }

export const useDepsStore = create<DepsState>((set, get) => ({
  ...NO_DEPS,

  open: (pkg) => {
    set({ focus: pkg })
  },
  refocus: (pkg) => {
    set({ focus: pkg })
  },
  close: () => {
    // The graph is kept. Leaving the view does not make the environment's edges wrong, and
    // dropping them here would make re-opening a row pay the probe again.
    set({ focus: null })
  },

  load: async (env, envHash) => {
    const { phase, graphFor } = get()
    if (phase === 'loading') return
    // Already have this environment's graph. Not a cache so much as the answer to "never render a
    // state you have not loaded" read the other way: a second fetch would replace an identical
    // value and flash a loading state over a view that was already correct.
    if (graphFor === envHash && get().graph !== null) return

    set({ phase: 'loading', error: null })
    try {
      const graph = await depsGraph(env)
      set({ graph, graphFor: envHash, phase: 'ready' })
    } catch (e) {
      // The graph is dropped, not kept: a failure here means the probe could not read the
      // environment, so whatever is in hand describes an environment that may no longer exist.
      set({ graph: null, graphFor: null, error: asPdError(e), phase: 'failed' })
    }
  },

  reset: () => {
    set({ ...NO_DEPS })
  },
}))

/**
 * The graph, but only when it describes `envHash`.
 *
 * A plain `graph` read would show one environment's edges after the user switched to another —
 * the shape `freshReport` and `freshAudit` already guard against. A function rather than a hook
 * so a component can call it inside a render without adding a subscription.
 */
export const freshGraph = (
  s: Pick<DepsState, 'graph' | 'graphFor'>,
  envHash: string,
): DepsGraph | null => (s.graphFor === envHash ? s.graph : null)

/**
 * One package's node, or null when the graph has never heard of it.
 *
 * `null` is a real answer rather than an error: a package can leave the environment between the
 * fetch and the click that focuses it, and a view that threw there would turn a stale row into a
 * crash.
 */
export const nodeOf = (graph: DepsGraph | null, pkg: string | null): DepsNode | null =>
  graph === null || pkg === null ? null : (graph.nodes[pkg] ?? null)
