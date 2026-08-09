/**
 * The 🔒 chip on a pinned row — UI-SPEC §4, `--color-info`.
 *
 * Extracted from `PdPackageRow` because the Pins screen shows the same thing and a second
 * hand-written copy is how the two modes stop agreeing about which is which.
 */

import { useTranslation } from 'react-i18next'

import { PdBadge } from '@/components/PdBadge'
import type { Pin } from '@/ipc'

export function PdPinChip({ mode }: { mode: Pin['mode'] }) {
  const { t } = useTranslation()

  return (
    <PdBadge
      tone="info"
      glyph={'\u{1F512}\u{FE0E}'}
      // A Hold pin restates a version in every plan and an Exclude pin does not, so the chip has
      // to tell them apart — UI-SPEC §4 says only "a 🔒 chip".
      label={mode === 'exclude' ? t('packages.badge.pinned') : mode.hold.version}
      title={
        mode === 'exclude'
          ? t('packages.badge.pinnedDetail')
          : t('packages.badge.heldDetail', { version: mode.hold.version })
      }
    />
  )
}
