/**
 * The rollback preview — DATA-FLOW §8's "listed explicitly; user may proceed partially".
 *
 * S6's exit criterion in component form: the unrestorable entries appear **before** the confirm,
 * under their code, rather than turning up in the summary after the user has already agreed.
 */

import { render, screen, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { PdRollbackPreview } from '@/components/PdRollbackPreview'
import type { RollbackPreview } from '@/ipc'
import previewFixture from '@/test/fixtures/rollback_preview.json'

const PREVIEW = previewFixture as RollbackPreview

function section(name: string) {
  const el = document.querySelector(`[data-section="${name}"]`)
  if (el === null) throw new Error(`no ${name} section`)
  return el as HTMLElement
}

describe('PdRollbackPreview', () => {
  it('lists what cannot be restored, with its code, before anything runs', () => {
    render(<PdRollbackPreview preview={PREVIEW} />)

    const stuck = section('unrestorable')
    expect(within(stuck).getByText('PD-SNP-002')).toBeInTheDocument()
    // Verbatim freeze lines — an editable install and a direct URL, exactly as recorded.
    expect(within(stuck).getByText('-e C:\\src\\editable-lib')).toBeInTheDocument()
    expect(within(stuck).getByText(/local-wheel @ file:/)).toBeInTheDocument()
  })

  it('says which packages go and which come back, with the versions', () => {
    render(<PdRollbackPreview preview={PREVIEW} />)

    expect(within(section('uninstall')).getByText('httpx')).toBeInTheDocument()
    // At the snapshot's version, not a bare name: otherwise this reads as a plain reinstall.
    expect(within(section('install')).getByText('numpy==1.26.4')).toBeInTheDocument()
    expect(within(section('install')).getByText('requests==2.28.0')).toBeInTheDocument()
  })

  it('names the snapshot it would restore to', () => {
    render(<PdRollbackPreview preview={PREVIEW} />)
    expect(screen.getByText(PREVIEW.target.id)).toBeInTheDocument()
  })

  it('says there is nothing to do rather than showing three empty sections', () => {
    render(
      <PdRollbackPreview
        preview={{
          target: PREVIEW.target,
          restore: { uninstall: [], install: [] },
          unrestorable: [],
        }}
      />,
    )
    expect(screen.getByText(/already matches the snapshot/)).toBeInTheDocument()
    expect(document.querySelector('[data-section]')).toBeNull()
  })
})
