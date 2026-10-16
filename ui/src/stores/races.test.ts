/**
 * Answers that arrive after the user has moved on.
 *
 * Every store here awaits a command and then writes what came back. The audit found the same
 * defect in five of them: the write did not ask whether the question was still the one being asked,
 * so a slow answer for the environment the user had just left landed on top of the one they had
 * switched to. Each test holds a command open, moves on, and only then lets it answer.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import * as ipc from '@/ipc'
import type { Dist, EnvRow, PyEnv } from '@/ipc'
import { SEARCH_DEBOUNCE_MS, useDepsStore, useEnvStore, useIndexStore, usePlanStore } from '@/stores'
import { resetStore } from '@/test/stores'

vi.mock('@/ipc', async () => {
  const { ipcMock } = await import('@/test/ipc')
  return ipcMock()
})

const env = (name: string): PyEnv => ({
  interpreter: `C:\\${name}\\Scripts\\python.exe`,
  prefix: `C:\\${name}`,
  pythonVersion: '3.12.4',
  externallyManaged: false,
  hiddenUserSite: null,
  source: 'manual',
})

const row = (name: string): EnvRow => ({
  interpreter: env(name).interpreter,
  source: 'manual',
  envHash: `hash-${name}`,
  env: env(name),
})

const dist = (name: string): Dist => ({
  name,
  version: '1.0',
  requiresDist: [],
  requiresPython: null,
  sizeBytes: null,
})

/** A promise the test resolves when it chooses. */
function held<T>() {
  let release: (value: T) => void = () => undefined
  const promise = new Promise<T>((r) => {
    release = r
  })
  return { promise, release }
}

describe('a slow answer for an environment the user has left', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    resetStore(useEnvStore)
    useEnvStore.setState({ rows: [row('a'), row('b')], selected: env('a').interpreter })
  })

  it('does not land on the environment they switched to', async () => {
    const slow = held<Dist[]>()
    vi.mocked(ipc.pkgList).mockReturnValueOnce(slow.promise).mockResolvedValueOnce([dist('from-b')])
    const first = useEnvStore.getState().loadPackages()

    useEnvStore.getState().select(env('b').interpreter)
    await useEnvStore.getState().loadPackages()
    slow.release([dist('from-a')])
    await first

    const { packages, loadedFor } = useEnvStore.getState()
    expect(loadedFor).toBe(env('b').interpreter)
    expect(packages.map((p) => p.name)).toEqual(['from-b'])
  })

  it('does not put the left environment’s badges on the new one', async () => {
    const slow = held<{ name: string; current: string; latest: string }[]>()
    vi.mocked(ipc.pkgOutdated).mockReturnValueOnce(slow.promise)
    const first = useEnvStore.getState().loadOutdated()

    useEnvStore.getState().select(env('b').interpreter)
    slow.release([{ name: 'from-a', current: '1.0', latest: '2.0' }])
    await first

    expect(useEnvStore.getState().outdated).toEqual([])
    expect(useEnvStore.getState().outdatedStatus).toBe('idle')
  })
})

describe('the timeline', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    resetStore(useEnvStore)
    useEnvStore.setState({ rows: [row('a')], openFor: env('a').interpreter })
  })

  it('shows the diff of the entry last clicked, whichever answers first', async () => {
    const slow = held<{ added: never[]; removed: never[]; changed: never[] }>()
    const second = { added: [{ name: 'later', version: '2.0' }], removed: [], changed: [] }
    vi.mocked(ipc.snapshotDiff).mockReturnValueOnce(slow.promise).mockResolvedValueOnce(second)

    const first = useEnvStore.getState().selectSnapshot('one')
    await useEnvStore.getState().selectSnapshot('two')
    slow.release({ added: [], removed: [], changed: [] })
    await first

    expect(useEnvStore.getState().selectedSnapshot).toBe('two')
    expect(useEnvStore.getState().diff).toEqual(second)
  })
})

describe('leaving a plan the backend is still working on', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    resetStore(usePlanStore)
  })

  it('stops it there, and its late answer does not reopen the panel', async () => {
    const slow = held<never>()
    vi.mocked(ipc.planResolve).mockReturnValueOnce(slow.promise)
    const resolving = usePlanStore.getState().resolve(env('a'), { intent: 'install', specs: ['six'] })
    expect(usePlanStore.getState().phase).toBe('resolving')

    usePlanStore.getState().reset()
    expect(ipc.planCancel).toHaveBeenCalledTimes(1)

    slow.release({ step: 'needsConfirm', report: { changes: [], heldBack: [] } } as never)
    await resolving
    expect(usePlanStore.getState().phase).toBe('idle')
  })

  it('does not cancel a run whose summary the user is leaving', () => {
    usePlanStore.setState({ phase: 'summary' })
    usePlanStore.getState().reset()
    expect(ipc.planCancel).not.toHaveBeenCalled()
  })
})

describe('the dependency view', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    resetStore(useDepsStore)
  })

  it('loads the environment switched to while the last one was still loading', async () => {
    const slow = held<{ nodes: Record<string, never> }>()
    const forB = { nodes: { six: { dependents: [], dependencies: [] } } } as never
    vi.mocked(ipc.depsGraph).mockReturnValueOnce(slow.promise).mockResolvedValueOnce(forB)

    const first = useDepsStore.getState().load(env('a'), 'hash-a')
    await useDepsStore.getState().load(env('b'), 'hash-b')
    slow.release({ nodes: {} })
    await first

    expect(ipc.depsGraph).toHaveBeenCalledTimes(2)
    expect(useDepsStore.getState().graphFor).toBe('hash-b')
    expect(useDepsStore.getState().graph).toEqual(forB)
  })
})

describe('search while the index is still loading', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.useFakeTimers()
    resetStore(useIndexStore)
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  it('asks again rather than waiting for another keystroke', async () => {
    vi.mocked(ipc.indexSearch)
      .mockResolvedValueOnce({ hits: [], ready: false })
      .mockResolvedValueOnce({ hits: [{ name: 'requests', display: 'requests', kind: 'exact' }], ready: true } as never)

    useIndexStore.getState().setQuery('requests')
    // Past the debounce, whichever edge fired: the first answer is "still warming".
    await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS + 1)
    expect(useIndexStore.getState().ready).toBe(false)
    expect(ipc.indexSearch).toHaveBeenCalledTimes(1)

    await vi.advanceTimersByTimeAsync(200)
    expect(ipc.indexSearch).toHaveBeenCalledTimes(2)
    expect(useIndexStore.getState().ready).toBe(true)
    expect(useIndexStore.getState().hits).toHaveLength(1)
  })
})
