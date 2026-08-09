/**
 * Reset a zustand store between tests.
 *
 * The stores are module singletons created at import time, so without this the second test in a
 * file inherits whatever the first left behind — and the failure that produces is the worst kind:
 * it passes alone and fails in a suite, or the reverse, depending on file order.
 *
 * The initial state is captured on first call, which has to happen before anything mutates the
 * store. Calling it from a `beforeEach` does that by construction.
 */

const initial = new WeakMap<object, unknown>()

interface ResettableStore<T> {
  getState: () => T
  setState: (state: T, replace: true) => void
}

export function resetStore<T extends object>(store: ResettableStore<T>): void {
  if (!initial.has(store)) initial.set(store, { ...store.getState() })
  // `replace: true`, so a field a test added is removed rather than merged over.
  store.setState(initial.get(store) as T, true)
}
