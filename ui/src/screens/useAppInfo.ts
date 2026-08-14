/**
 * `app_info`, asked for **once per process** rather than once per mount.
 *
 * `app_info` is a `const fn` in Rust: the version and the legal-documents hash are baked in at
 * compile time and cannot change while the app is running, so a second call can only ever return
 * the first answer. And mounts are not rare — `App.tsx` puts `key={nav}` on `<main>`, so every
 * visit to About is a fresh mount and would be a fresh call. Every other screen gets its data from
 * a store that remembers what it already loaded; About has no store worth adding, so the promise
 * itself is the cache.
 *
 * Found by running rather than by the test. `PdAbout.test.tsx` asserted one call and passed, while
 * the bridge log against `npm run dev` showed two — `StrictMode` double-invokes effects in
 * development and Testing Library does not use it. The doubling is dev-only; the per-visit refetch
 * it drew attention to was not.
 *
 * It also does not reject: `app_info` is the one command in the surface returning `AppInfo` rather
 * than a `Result`, so there is no error state here and none is invented.
 */

import { useEffect, useState } from 'react'

import { appInfo, type AppInfo } from '@/ipc'

let pending: Promise<AppInfo> | null = null

/** Null until the first answer arrives. Never renders a value it has not loaded. */
export function useAppInfo(): AppInfo | null {
  const [info, setInfo] = useState<AppInfo | null>(null)

  useEffect(() => {
    let alive = true
    pending ??= appInfo()
    void pending.then((i) => {
      if (alive) setInfo(i)
    })
    return () => {
      alive = false
    }
  }, [])

  return info
}

/** Test-only: drop the cache, so a case can observe the fetch instead of a previous case's. */
export function resetAppInfoCache(): void {
  pending = null
}
