/**
 * Load the open environment's snapshots, once per environment.
 *
 * The sibling of `useEnvPackages`, and separate from it because the two are read by different
 * screens for different reasons: nothing but the detail view wants a timeline, and paying a
 * directory read on every Installed mount to have it ready would be a fetch nobody asked for.
 */

import { useEffect } from 'react'

import { useEnvStore } from '@/stores'

export function useEnvSnapshots(): void {
  const openFor = useEnvStore((s) => s.openFor)
  const loadSnapshots = useEnvStore((s) => s.loadSnapshots)

  useEffect(() => {
    if (openFor !== null) void loadSnapshots()
  }, [openFor, loadSnapshots])
}
