/**
 * Live engine output — UI-SPEC §3's console drawer.
 *
 * "Slides up over the status line during execution; collapsible, **never modal**." Never modal
 * matters: this is the thing a user opens *because* something is taking a long time, and a modal
 * would stop them cancelling it.
 *
 * Sections come from the `plan-progress` lifecycle rather than from the text. Grouping by parsing
 * output was the thing that could not be done before `stepStarted` existed — engine output does
 * not reliably say which package it concerns.
 */

import { useEffect, useRef } from 'react'
import { useTranslation } from 'react-i18next'

import type { ConsoleLine } from '@/stores'

interface PdConsoleDrawerProps {
  lines: readonly ConsoleLine[]
  open: boolean
  onClose: () => void
  /** Shown in the header, and announced: UI-SPEC §8's "13 of 15 complete". */
  done: number
  total: number
}

/** Group consecutive lines by step, so each package gets one heading. */
function sections(lines: readonly ConsoleLine[]): { key: string; pkg?: string; lines: string[] }[] {
  const out: { key: string; pkg?: string; lines: string[] }[] = []
  for (const line of lines) {
    const last = out.at(-1)
    if (last === undefined || last.key !== `${String(line.step)}:${line.pkg ?? ''}`) {
      out.push({
        key: `${String(line.step)}:${line.pkg ?? ''}`,
        ...(line.pkg === undefined ? {} : { pkg: line.pkg }),
        lines: [line.text],
      })
    } else {
      last.lines.push(line.text)
    }
  }
  return out
}

export function PdConsoleDrawer({ lines, open, onClose, done, total }: PdConsoleDrawerProps) {
  const { t } = useTranslation()
  const bottom = useRef<HTMLDivElement>(null)

  useEffect(() => {
    // Follow the tail. A user watching a build wants the newest line, not the first.
    bottom.current?.scrollIntoView({ block: 'end' })
  }, [lines.length])

  if (!open) return null

  return (
    <section
      aria-label={t('plan.console')}
      className="flex max-h-72 shrink-0 flex-col border-t border-border bg-surface"
      onKeyDown={(e) => {
        // UI-SPEC §8: Esc closes drawers.
        if (e.key === 'Escape') onClose()
      }}
    >
      <div className="flex shrink-0 items-center justify-between border-b border-border px-3 py-1">
        <span className="text-data text-text-dim">{t('plan.console')}</span>
        {/* **Not a live region.** It was one, and so is the screen that opens this drawer — both
            announcing the same `done/total`, so a reader heard every step twice. §8 asks for one
            announcement of execution progress, and the screen owns it because the drawer may be
            closed while a run is going. Still rendered, still read on demand. */}
        <span className="font-mono text-data text-text-dim">
          {total > 0 ? t('plan.progress', { done, total }) : ''}
        </span>
        <button
          type="button"
          onClick={onClose}
          aria-label={t('actions.cancel')}
          className="rounded-pd border border-border px-2 py-0.5 text-data"
        >
          {'✕'}
        </button>
      </div>

      <div className="min-h-0 flex-1 overflow-auto px-3 py-2">
        {sections(lines).map((section) => (
          <div key={section.key} className="mb-2">
            {section.pkg === undefined ? null : (
              <p className="font-mono text-data text-accent-dim">{`▸ ${section.pkg}`}</p>
            )}
            <pre className="font-mono text-data whitespace-pre-wrap text-text-dim">
              {section.lines.join('\n')}
            </pre>
          </div>
        ))}
        <div ref={bottom} />
      </div>
    </section>
  )
}
