/**
 * The language a user chose is still their language after a restart — U3.
 *
 * The locale radio applied a language live and stored nothing: `save` sent `locale: null` every
 * time, so choosing Vietnamese lasted until the window closed, and saving any other setting
 * afterwards would have erased a choice even if one had been written. `load` ignored the stored
 * value as well, and the radio started on English whatever the OS was.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import i18n from '@/i18n'
import * as ipc from '@/ipc'
import type { Settings } from '@/ipc'
import { useSettingsStore } from '@/stores'
import { resetStore } from '@/test/stores'

vi.mock('@/ipc', async () => {
  const { ipcMock } = await import('@/test/ipc')
  return ipcMock()
})

const STORED: Settings = {
  engine: 'pip',
  locale: null,
  allowExternallyManaged: false,
  pinSuggestThreshold: 5,
}

describe('the locale', () => {
  beforeEach(async () => {
    vi.clearAllMocks()
    resetStore(useSettingsStore)
    vi.mocked(ipc.settingsSet).mockImplementation((s: Settings) => Promise.resolve(s))
    await i18n.changeLanguage('en')
  })
  afterEach(async () => {
    await i18n.changeLanguage('en')
  })

  it('applies the stored choice when settings load', async () => {
    vi.mocked(ipc.settingsGet).mockResolvedValue({ ...STORED, locale: 'vi' })
    await useSettingsStore.getState().load()
    expect(useSettingsStore.getState().locale).toBe('vi')
    expect(i18n.language).toBe('vi')
  })

  it('follows the language already in use when nothing was chosen', async () => {
    vi.mocked(ipc.settingsGet).mockResolvedValue(STORED)
    await useSettingsStore.getState().load()
    expect(useSettingsStore.getState().locale).toBe('en')
    expect(useSettingsStore.getState().localeChoice).toBeNull()
  })

  it('stores a choice when one is made', async () => {
    vi.mocked(ipc.settingsGet).mockResolvedValue(STORED)
    await useSettingsStore.getState().load()
    await useSettingsStore.getState().setLocale('vi')

    expect(i18n.language).toBe('vi')
    expect(ipc.settingsSet).toHaveBeenLastCalledWith(expect.objectContaining({ locale: 'vi' }))
  })

  it('keeps the choice when another setting is saved', async () => {
    // The second half of the bug: `save` wrote `locale: null` with every other change.
    vi.mocked(ipc.settingsGet).mockResolvedValue({ ...STORED, locale: 'vi' })
    await useSettingsStore.getState().load()
    await useSettingsStore.getState().save({ engine: 'uv' })

    expect(ipc.settingsSet).toHaveBeenLastCalledWith(
      expect.objectContaining({ engine: 'uv', locale: 'vi' }),
    )
  })

  it('does not invent a choice for someone who never made one', async () => {
    // `null` means "follow the OS". Writing the resolved language back would pin it.
    vi.mocked(ipc.settingsGet).mockResolvedValue(STORED)
    await useSettingsStore.getState().load()
    await useSettingsStore.getState().save({ engine: 'uv' })

    expect(ipc.settingsSet).toHaveBeenLastCalledWith(expect.objectContaining({ locale: null }))
  })
})
