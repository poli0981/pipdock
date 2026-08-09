/**
 * Load the selected environment's packages on mount, once per environment.
 *
 * Three screens need the installed set for different reasons — Installed and Updates render it,
 * Search needs it for DATA-FLOW §4's `INSTALLED`/`UPDATE` chips, and Pins joins pins against it —
 * and each of them can be the first screen a user opens. The effect was written twice already and
 * a third copy is how they start disagreeing.
 *
 * **Both fetches, always.** `loadedFor` is one flag covering two calls, so a screen that loads only
 * `pkg_list` sets it and the outdated fetch never happens for that environment: every row would
 * stay `unknown`, permanently. That is the whole reason this is one hook rather than a convention.
 *
 * The `loadedFor` guard is also what stops React's double-invoked development effects fetching
 * twice, and what makes switching between Installed and Updates free.
 */

import { useEffect } from 'react'

import { useEnvStore } from '@/stores'

export function useEnvPackages(): void {
  const selected = useEnvStore((s) => s.selected)
  const loadedFor = useEnvStore((s) => s.loadedFor)
  const loadPackages = useEnvStore((s) => s.loadPackages)
  const loadOutdated = useEnvStore((s) => s.loadOutdated)

  useEffect(() => {
    if (selected !== null && loadedFor !== selected) {
      void loadPackages()
      void loadOutdated()
    }
  }, [selected, loadedFor, loadPackages, loadOutdated])
}
