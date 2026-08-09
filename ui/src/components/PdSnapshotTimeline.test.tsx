/**
 * The timeline — UI-SPEC §4's "timeline of snapshots with trigger label".
 *
 * The trigger label is the point. A restore writes its own snapshot before restoring, so `latest`
 * moves twice across one rollback; the labels are how a user tells the entry they want from the
 * one the rollback just created.
 */

import { fireEvent, render, screen, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { PdSnapshotTimeline } from '@/components/PdSnapshotTimeline'
import type { SnapshotMeta } from '@/ipc'
import snapshotFixture from '@/test/fixtures/snapshot_list.json'

const SNAPSHOTS = snapshotFixture as SnapshotMeta[]

function setup(overrides: { selected?: string | null; selectable?: boolean } = {}) {
  const props = {
    snapshots: SNAPSHOTS,
    selected: overrides.selected ?? null,
    selectable: overrides.selectable ?? true,
    onSelect: vi.fn(),
  }
  render(<PdSnapshotTimeline {...props} />)
  return props
}

/** One timeline entry by snapshot id. */
function entry(id: string) {
  const el = document.querySelector(`[data-snapshot="${id}"]`)
  if (el === null) throw new Error(`no timeline entry for ${id}`)
  return el as HTMLElement
}

describe('PdSnapshotTimeline', () => {
  it('tells the three triggers apart', () => {
    setup()
    expect(within(entry('20260809T140000-0000000Z')).getByText(/before restoring/)).toBeTruthy()
    expect(within(entry('20260809T130000-0000000Z')).getByText('before a change')).toBeTruthy()
    expect(within(entry('20260809T120000-0000000Z')).getByText('taken by you')).toBeTruthy()
  })

  it('names the snapshot a rollback entry was restoring', () => {
    // Without it, two adjacent entries a minute apart are indistinguishable — and one of them is
    // the state the user is trying to get back to while the other is the state they are leaving.
    setup()
    expect(
      within(entry('20260809T140000-0000000Z')).getByText(/20260809T120000-0000000Z/),
    ).toBeTruthy()
  })

  it('shows every id verbatim, because that is what the commands take', () => {
    setup()
    for (const meta of SNAPSHOTS) {
      expect(screen.getByText(meta.id)).toBeInTheDocument()
    }
  })

  it('hands back the id it was clicked on', () => {
    const { onSelect } = setup()
    fireEvent.click(entry('20260809T130000-0000000Z'))
    expect(onSelect).toHaveBeenCalledWith('20260809T130000-0000000Z')
  })

  it('marks the selected entry for assistive tech, not only visually', () => {
    setup({ selected: '20260809T130000-0000000Z' })
    expect(entry('20260809T130000-0000000Z')).toHaveAttribute('aria-current', 'true')
    expect(entry('20260809T120000-0000000Z')).toHaveAttribute('aria-current', 'false')
  })

  it('lists but does not offer an environment that cannot be read', () => {
    // Snapshots outlive the interpreter that made them, so the history is still worth showing —
    // but nothing can be diffed against a Python that is gone.
    setup({ selectable: false })
    for (const meta of SNAPSHOTS) {
      expect(entry(meta.id)).toBeDisabled()
    }
  })
})
