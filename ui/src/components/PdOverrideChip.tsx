/**
 * The persistent warning SECURITY §3 requires while the PEP 668 override applies to an
 * environment.
 *
 * Shown wherever a change to that environment can start — Installed, Search's dock bay, the plan
 * panel, an environment's detail — and **only** where it applies: the override changes nothing for
 * a Python without the `EXTERNALLY-MANAGED` marker, and a chip there would claim it did. Before
 * 1.4.0 the override was a checkbox that saved and nothing read, and no chip existed anywhere.
 */

import { useTranslation } from 'react-i18next'

import { PdBadge } from '@/components/PdBadge'
import type { PyEnv } from '@/ipc'
import { useSettingsStore } from '@/stores'

export function PdOverrideChip({ env }: { env: PyEnv | null | undefined }) {
  const { t } = useTranslation()
  const allow = useSettingsStore((s) => s.allowExternallyManaged)
  if (!allow || env?.externallyManaged !== true) return null
  return (
    <PdBadge tone="danger" glyph={'⚠︎'} label={t('pep668.chip')} title={t('pep668.chipDetail')} />
  )
}
