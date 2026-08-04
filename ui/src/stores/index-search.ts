/**
 * Search and the dock bay — ARCHITECTURE §9's `useIndexStore`.
 *
 * The whole screen exists under one number: **< 50 ms per keystroke**, measured in the app. SP-3
 * measured 42.1 ms for the ranking alone, in memory, with no IPC round trip — this adds one, so
 * the design has to assume the budget is tight rather than discover it afterwards. Three things
 * follow, and all three are here rather than in the component:
 *
 * 1. **The previous result set stays rendered** while a new query is in flight. Blanking the list
 *    on every keystroke is what makes a fast search *feel* slow. Measured: ranking costs 16 ms
 *    worst case in release, so most of the perceived latency is whatever the UI adds.
 * 2. **Responses are sequenced.** Two searches can be in flight and they can return out of order;
 *    without a sequence number the older one wins and the list contradicts the field.
 * 3. **A debounce, but a short one.** Long enough to skip work while a fast typist is mid-word,
 *    short enough to be invisible — and it is a *coalescer*, not a delay: the first keystroke
 *    after a pause searches immediately.
 */

import { create } from 'zustand'

import {
  indexRefresh,
  indexSearch,
  isPdError,
  pkgMetadata,
  type Freshness,
  type Hit,
  type PackageMeta,
  type PdError,
} from '@/ipc'

/**
 * Coalescing window, in ms.
 *
 * One frame. Anything longer is felt as lag on a budget this tight; anything shorter stops
 * coalescing anything at all. The debounce exists to avoid queueing work behind a fast typist,
 * not to wait for them to stop.
 */
export const SEARCH_DEBOUNCE_MS = 16

/**
 * How many hits to ask for.
 *
 * Was 50, and **measured**: at 50 rows the keystroke-to-painted time was a 57 ms median against a
 * 50 ms budget, because React commits every row on every keystroke. 20 is a screenful — the list
 * scrolls, and nobody reads past the first few of a fuzzy match — and it is the single largest
 * lever on the render half of the budget.
 */
export const SEARCH_LIMIT = 20

/** One queued package — UI-SPEC §4's "dock bay". */
export interface QueuedPackage {
  name: string
  /** Empty means "latest", which is what the resolver does with no version specifier. */
  version: string
}

interface IndexState {
  query: string
  hits: Hit[]
  /** False while the index load is still running (~140 ms in release). */
  ready: boolean
  /** Set when the index cannot be loaded at all — the screen offers a refresh. */
  unavailable: string | null
  /** True while a search is in flight, for a spinner that does not blank the list. */
  searching: boolean
  error: PdError | null

  /** The details panel's subject. */
  selected: string | null
  meta: PackageMeta | null
  metaFreshness: Freshness | null
  metaError: PdError | null

  /** The dock bay queue, in the order packages were added. */
  queue: QueuedPackage[]

  refreshing: boolean
  lastRefresh: string | null

  setQuery: (query: string) => void
  select: (name: string) => Promise<void>
  enqueue: (name: string) => void
  setQueuedVersion: (name: string, version: string) => void
  dequeue: (name: string) => void
  clearQueue: () => void
  refreshIndex: () => Promise<void>
}

function asPdError(e: unknown): PdError {
  return isPdError(e) ? e : { code: 'PD-INT-001', message: String(e) }
}

/**
 * Sequence of the most recent search, and of the most recent one applied.
 *
 * Module-level rather than store state: they are bookkeeping no component renders, and putting
 * them in the store would wake every subscriber on each keystroke.
 */
let issued = 0
let applied = 0
let timer: ReturnType<typeof setTimeout> | undefined
/** When the last search actually fired, for the leading edge below. */
let lastFired = 0

export const useIndexStore = create<IndexState>((set, get) => ({
  query: '',
  hits: [],
  ready: true,
  unavailable: null,
  searching: false,
  error: null,
  selected: null,
  meta: null,
  metaFreshness: null,
  metaError: null,
  queue: [],
  refreshing: false,
  lastRefresh: null,

  setQuery: (query) => {
    set({ query })

    if (timer !== undefined) clearTimeout(timer)
    if (query.trim() === '') {
      // Clearing the field clears the list immediately — there is nothing to keep showing.
      issued += 1
      applied = issued
      set({ hits: [], searching: false })
      return
    }

    const fire = () => {
      lastFired = performance.now()
      const seq = (issued += 1)
      set({ searching: true })
      indexSearch(query, SEARCH_LIMIT)
        .then((results) => {
          // Out-of-order arrival: a slower earlier search must not overwrite a later one.
          if (seq < applied) return
          applied = seq
          set({
            hits: results.hits,
            ready: results.ready,
            unavailable: results.unavailable ?? null,
            searching: false,
            error: null,
          })
        })
        .catch((e: unknown) => {
          if (seq < applied) return
          applied = seq
          set({ searching: false, error: asPdError(e) })
        })
    }

    // **Leading edge.** The debounce is a coalescer, not a delay: the first keystroke after a
    // pause searches at once, and only a burst is batched. Measured — a trailing-only debounce
    // spent 16 ms of a 50 ms budget doing nothing on every single keystroke, which is a third of
    // it. The comment above claimed this behaviour before the code did.
    if (performance.now() - lastFired >= SEARCH_DEBOUNCE_MS) {
      fire()
      return
    }
    timer = setTimeout(fire, SEARCH_DEBOUNCE_MS)
  },

  select: async (name) => {
    set({ selected: name, meta: null, metaFreshness: null, metaError: null })
    try {
      const [meta, freshness] = await pkgMetadata(name)
      // The user may have moved on while PyPI was answering.
      if (get().selected !== name) return
      set({ meta, metaFreshness: freshness })
    } catch (e) {
      if (get().selected !== name) return
      set({ metaError: asPdError(e) })
    }
  },

  enqueue: (name) =>
    set((s) =>
      s.queue.some((q) => q.name === name) ? {} : { queue: [...s.queue, { name, version: '' }] },
    ),

  setQueuedVersion: (name, version) =>
    set((s) => ({
      queue: s.queue.map((q) => (q.name === name ? { ...q, version } : q)),
    })),

  dequeue: (name) => set((s) => ({ queue: s.queue.filter((q) => q.name !== name) })),
  clearQueue: () => set({ queue: [] }),

  refreshIndex: async () => {
    set({ refreshing: true, error: null })
    try {
      const report = await indexRefresh()
      set({
        refreshing: false,
        unavailable: null,
        ready: true,
        lastRefresh: String(report.projects),
      })
      // The in-memory index was invalidated on the Rust side, so re-run whatever is in the field.
      const { query, setQuery } = get()
      if (query.trim() !== '') setQuery(query)
    } catch (e) {
      set({ refreshing: false, error: asPdError(e) })
    }
  },
}))

/** The `name==version` specs the dock bay would install, as `Intent::Install` wants them. */
export function queueSpecs(queue: readonly QueuedPackage[]): string[] {
  return queue.map((q) => (q.version.trim() === '' ? q.name : `${q.name}==${q.version.trim()}`))
}
