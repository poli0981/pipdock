/**
 * The guard dialog — DATA-FLOW §5's three options, and UI-SPEC §7's focus rule.
 *
 * The `GuardReport` is the **generated** fixture, computed in Rust by running the real graph over
 * the real `pkg_list()` scenario. Hand-writing one would let the guard's own rules drift away from
 * what this asserts while both stayed green.
 */

import { fireEvent, render, screen, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { PdUninstallDialog } from '@/components/PdUninstallDialog'
import type { GuardReport } from '@/ipc'
import guardFixture from '@/test/fixtures/guard_report.json'

const BREAKING = guardFixture as GuardReport
/** The same removal with nothing depending on it. */
const CLEAR: GuardReport = { removing: ['certifi'], breaks: {}, withDependents: ['certifi'] }

function setup(report: GuardReport, overrides: { busy?: boolean } = {}) {
  const props = {
    report,
    busy: overrides.busy ?? false,
    onCancel: vi.fn(),
    onWiden: vi.fn(),
    onConfirm: vi.fn(),
  }
  render(<PdUninstallDialog {...props} />)
  return props
}

/** jsdom implements `showModal`, so the dialog's own subtree is where we look. */
function dialog() {
  return screen.getByRole('dialog')
}

describe('PdUninstallDialog', () => {
  it('names each dependent and the constraint it declared', () => {
    // The slice's exit criterion, and the reason `GuardReport.breaks` carries specifiers at all: a
    // list of names says what breaks and not whether the user can live with it.
    setup(BREAKING)

    expect(screen.getByText('pandas 2.1.4 requires numpy<2,>=1.26.0')).toBeInTheDocument()
    expect(screen.getByText('scipy 1.11.4 requires numpy<1.28.0,>=1.21.6')).toBeInTheDocument()
  })

  it('focuses Cancel, and renders it first', () => {
    // UI-SPEC §7: destructive confirms "require the dialog's default focus to be Cancel".
    setup(BREAKING)
    const cancel = within(dialog()).getByText('Cancel')

    expect(cancel).toHaveFocus()
    const buttons = within(dialog()).getAllByRole('button')
    expect(buttons[0]).toBe(cancel)
  })

  it('re-guards rather than proceeding when the dependents are added', () => {
    // The other exit criterion. Widening must go back through the guard, because pulling one
    // dependent in can break a third package — proceeding here is the bare-`pip uninstall`
    // behaviour the guard exists to replace.
    const { onWiden, onConfirm } = setup(BREAKING)
    fireEvent.click(within(dialog()).getByText(/Remove the dependents too/))

    expect(onWiden).toHaveBeenCalledOnce()
    expect(onConfirm).not.toHaveBeenCalled()
  })

  it('forces only when the user chose to break things', () => {
    const { onConfirm } = setup(BREAKING)
    fireEvent.click(within(dialog()).getByText(/Remove anyway/))
    expect(onConfirm).toHaveBeenCalledWith(true)
  })

  it('does not force a removal that breaks nothing', () => {
    // `force` is an acknowledgement of breakage, not a synonym for confirm. Sending `true` here
    // would waive a guard that never objected.
    const { onConfirm } = setup(CLEAR)
    expect(within(dialog()).queryByText(/Remove anyway/)).not.toBeInTheDocument()

    fireEvent.click(within(dialog()).getByText(/^Remove 1 package$/))
    expect(onConfirm).toHaveBeenCalledWith(false)
  })

  it('disables every control while a re-guard is in flight', () => {
    const { onWiden, onConfirm } = setup(BREAKING, { busy: true })
    for (const button of within(dialog()).getAllByRole('button')) {
      expect(button).toBeDisabled()
    }
    fireEvent.click(within(dialog()).getByText(/Remove anyway/))
    expect(onWiden).not.toHaveBeenCalled()
    expect(onConfirm).not.toHaveBeenCalled()
  })
})
