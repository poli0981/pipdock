/**
 * `PdPackageTable` dimming and badging — TESTING §2 names this as one of L3's five obligations,
 * and L3 is a PR-blocking gate (§3).
 *
 * The repo's first rendered-component test. It asserts on `data-state` and `data-pinned`, which
 * the row derives from the same expressions as its class names, rather than on Tailwind classes —
 * a restyle should not break this, a change to the *rule* should. One class assertion remains, for
 * the dimming token itself, since that is the rule UI-SPEC §4 states in terms of a colour.
 */

// `fireEvent` rather than `user-event`: these are plain onKeyDown handlers, and a devDependency
// that has to clear `npm audit --audit-level=high` forever needs a better reason than convenience.
import { fireEvent, render, screen, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { PdPackageTable } from '@/components/PdPackageTable'
import type { Pin } from '@/ipc'
import { joinRows, type LoadState, type PackageRow } from '@/screens/rows'
import pinFixture from '@/test/fixtures/pin_list.json'
import listFixture from '@/test/fixtures/pkg_list.json'
import outdatedFixture from '@/test/fixtures/pkg_outdated.json'

// Only the pins need a cast: `PinMode` is a union, and JSON import inference widens
// `"exclude"` to `string`. The other two fixtures type-check as they stand, which is itself a
// small check that they still match the generated types.
const { rows: ROWS } = joinRows(listFixture, outdatedFixture, pinFixture as Pin[])

function setup(overrides: Partial<Parameters<typeof PdPackageTable>[0]> = {}) {
  const props = {
    rows: ROWS,
    outdatedStatus: 'ready' as LoadState,
    selection: new Set<string>(),
    onToggle: vi.fn(),
    onPinToggle: vi.fn(),
    onUninstall: vi.fn(),
    onDetails: vi.fn(),
    onSelectAll: vi.fn(),
    initialRect: { width: 1280, height: 600 },
    ...overrides,
  }
  render(<PdPackageTable {...props} />)
  return props
}

/** A rendered row by package name. */
function rowFor(name: string) {
  const cell = screen.getByText(name)
  const row = cell.closest('[role="row"]')
  if (row === null) throw new Error(`no row rendered for ${name}`)
  return row
}

describe('dimming and badging', () => {
  it('dims an up-to-date row and gives it no badge', () => {
    setup()
    const certifi = rowFor('certifi')
    expect(certifi).toHaveAttribute('data-state', 'current')
    expect(certifi).toHaveClass('text-text-dim')
    expect(within(certifi as HTMLElement).queryByText('UPDATE')).toBeNull()
  })

  it('badges an outdated row and shows the version it could move to', () => {
    setup()
    const pandas = rowFor('pandas')
    expect(pandas).toHaveAttribute('data-state', 'outdated')
    expect(pandas).not.toHaveClass('text-text-dim')
    expect(within(pandas as HTMLElement).getByText('UPDATE')).toBeInTheDocument()
    expect(within(pandas as HTMLElement).getByText('2.3.0')).toBeInTheDocument()
  })

  it('dims nothing and badges nothing while the outdated set is still loading', () => {
    // The assertion that catches the flash: treating "not in the outdated set" as "up to date"
    // would dim all of these and un-dim a handful a second later.
    setup({ outdatedStatus: 'loading' })
    expect(screen.queryAllByText('UPDATE')).toHaveLength(0)
    for (const row of screen.getAllByRole('row').slice(1)) {
      expect(row).toHaveAttribute('data-state', 'unknown')
      expect(row).not.toHaveClass('text-text-dim')
    }
  })

  it('renders an em dash rather than a number when the size is unknowable', () => {
    setup()
    // The editable install: its RECORD lists the import shim, so the probe reports nothing.
    expect(within(rowFor('editable-lib') as HTMLElement).getAllByText('—').length).toBeGreaterThan(0)
    expect(within(rowFor('numpy') as HTMLElement).getByText('60.0 MiB')).toBeInTheDocument()
  })
})

describe('pins', () => {
  it('marks a pinned row and refuses to select it', () => {
    setup()
    const scipy = rowFor('scipy')
    expect(scipy).toHaveAttribute('data-pinned', 'true')
    expect(within(scipy as HTMLElement).getByRole('checkbox')).toBeDisabled()
  })

  it('tells a held pin apart from an excluded one, which UI-SPEC does not', () => {
    setup()
    // An Exclude pin says only "pinned"; a Hold restates a version in every plan, so its chip
    // names the version. Matched by title, not text: a hold is usually at the package's current
    // version, so "1.26.4" legitimately appears in both the version cell and the chip.
    expect(
      within(rowFor('scipy') as HTMLElement).getByTitle(/left out of bulk updates/i),
    ).toHaveTextContent('pinned')
    expect(within(rowFor('numpy') as HTMLElement).getByTitle(/^Held at 1\.26\.4/)).toBeInTheDocument()
  })
})

describe('virtualization', () => {
  it('renders a window, not the whole list', () => {
    // Without this the suite is vacuous in the other direction too: jsdom reports zero height,
    // so an unconfigured virtualizer renders no rows and every content assertion above would
    // pass by having nothing to contradict it.
    const many: PackageRow[] = Array.from({ length: 500 }, (_, i) => ({
      name: `pkg-${String(i).padStart(3, '0')}`,
      version: '1.0.0',
    }))
    setup({ rows: many })

    const grid = screen.getByRole('grid')
    expect(grid).toHaveAttribute('aria-rowcount', '501')

    const rendered = screen.getAllByRole('row').length - 1
    expect(rendered).toBeGreaterThan(0)
    expect(rendered).toBeLessThan(many.length)
  })

  it('numbers rows against the full list, not the rendered window', () => {
    setup()
    // Otherwise a screen reader announces "row 3 of 25" in a 200-package environment.
    expect(rowFor('certifi')).toHaveAttribute('aria-rowindex', '2')
  })
})

describe('keyboard', () => {
  it('toggles the focused row on Space', () => {
    const { onToggle } = setup()
    fireEvent.keyDown(rowFor('pandas'), { key: ' ' })
    expect(onToggle).toHaveBeenCalledWith('pandas')
  })

  it('leaves a pinned row alone on Space', () => {
    const { onToggle } = setup()
    fireEvent.keyDown(rowFor('scipy'), { key: ' ' })
    expect(onToggle).not.toHaveBeenCalled()
  })

  it('makes the row the tab stop, not its checkbox or its actions', () => {
    // 200 rows × four tab stops each is not keyboard traversal. The pin button had no tabIndex,
    // so it *was* one — and Space on it toggled the row's selection instead of pinning.
    setup()
    const row = rowFor('pandas') as HTMLElement
    expect(row).toHaveAttribute('tabindex', '0')
    expect(within(row).getByRole('checkbox')).toHaveAttribute('tabindex', '-1')
    for (const button of within(row).getAllByRole('button')) {
      expect(button).toHaveAttribute('tabindex', '-1')
    }
  })

  it('does not toggle selection when Space is pressed on a row action', () => {
    const { onToggle, onUninstall } = setup()
    const remove = within(rowFor('pandas') as HTMLElement).getByLabelText('Remove this package')

    // The event still bubbles to the row's handler; what stops it acting is the target check.
    fireEvent.keyDown(remove, { key: ' ', bubbles: true })
    expect(onToggle).not.toHaveBeenCalled()
    expect(onUninstall).not.toHaveBeenCalled()
  })

  it('offers removal from the row, which is the first of the 3 clicks', () => {
    const { onUninstall } = setup()
    fireEvent.click(within(rowFor('pandas') as HTMLElement).getByLabelText('Remove this package'))
    expect(onUninstall).toHaveBeenCalledWith('pandas')
  })

  it('selects all on Ctrl+A', () => {
    const { onSelectAll } = setup()
    fireEvent.keyDown(rowFor('pandas'), { key: 'a', ctrlKey: true })
    expect(onSelectAll).toHaveBeenCalledOnce()
  })
})
