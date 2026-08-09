/**
 * A modal confirm — UI-SPEC §7's "destructive confirms … require the dialog's default focus to be
 * **Cancel**".
 *
 * The repo's first real dialog. `PdConflictRow` confirms *Force latest* inline, which works there
 * because the thing being confirmed is one row of a list that stays put. A removal's confirm
 * cannot: it is opened from a row inside a virtualized table, and `PdPackageTable` renders a ~25
 * row window — scroll, and the element the dialog was mounted inside is unmounted underneath it.
 *
 * A native `<dialog>` with `showModal()` rather than a hand-built overlay, because the browser
 * already implements the parts that are easy to get wrong: the top layer (so no z-index fight),
 * the inert backdrop, focus containment, and Esc. What is left is the policy:
 *
 * - **Cancel is rendered first and takes focus**, per §7. `autoFocus` inside a `<dialog>` is
 *   honoured by `showModal()`, which focuses the first focusable element otherwise.
 * - **The backdrop does not dismiss.** Clicking beside a dialog that is about to delete things is
 *   not consent, and a mis-click that silently cancels is a smaller problem than one that does not.
 * - **`busy` disables everything** rather than unmounting, so a re-guard in flight cannot be
 *   double-submitted and the user keeps reading what they were reading.
 */

import { useEffect, useRef } from 'react'

interface PdDialogProps {
  /** Accessible name — the package or operation the dialog is about. */
  label: string
  /** Heading text, already localized. */
  title: string
  /** Body: the explanation, the list, whatever the caller needs. */
  children: React.ReactNode
  /** Cancel's label. Rendered first, focused on open. */
  cancelLabel: string
  onCancel: () => void
  /** Everything other than Cancel, in the order UI-SPEC §5 lists them. */
  actions: React.ReactNode
  /** While true every control is disabled: work is in flight and the answer would be stale. */
  busy?: boolean
}

export function PdDialog({
  label,
  title,
  children,
  cancelLabel,
  onCancel,
  actions,
  busy = false,
}: PdDialogProps) {
  const ref = useRef<HTMLDialogElement>(null)

  useEffect(() => {
    const el = ref.current
    // `showModal` throws if it is already open, which React's double-invoked effects in
    // development make a real case rather than a theoretical one.
    if (el !== null && !el.open) el.showModal()
    return () => {
      if (el !== null && el.open) el.close()
    }
  }, [])

  return (
    <dialog
      ref={ref}
      aria-label={label}
      // Esc reaches this rather than closing the element behind our back, so cancelling always
      // goes through the same path as the button.
      onCancel={(e) => {
        e.preventDefault()
        if (!busy) onCancel()
      }}
      className="max-w-lg rounded-pd border border-border bg-surface p-6 text-text backdrop:bg-bg/80"
    >
      <h2 className="text-accent">{title}</h2>
      <div className="mt-3 text-data">{children}</div>
      <div className="mt-4 flex flex-wrap gap-2">
        {/* First, and focused: UI-SPEC §7. The safe answer must be the one a reflex reaches. */}
        <button
          type="button"
          autoFocus
          disabled={busy}
          onClick={onCancel}
          className="rounded-pd border border-border px-3 py-1 text-data disabled:opacity-40"
        >
          {cancelLabel}
        </button>
        {actions}
      </div>
    </dialog>
  )
}
