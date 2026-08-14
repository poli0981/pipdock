/**
 * About — UI-SPEC §4.
 *
 * What PipDock is, which build this is, and how to reach its author. It also gives *Legal & About*
 * the home §4 promised it in Settings and never got: four milestones shipped with no in-app path
 * back to the documents the user accepted on first run, because a read-only surface folded into a
 * screen of controls is the thing that never gets built.
 *
 * Three constants below are **data, not copy** (I18N §2): an address, a URL and an SPDX identifier
 * are not translated and must not be reachable by a translator. Rendering them as `{CONTACT.…}`
 * rather than as JSX text is also what keeps `pipdock/no-jsx-literals` correct about them — the
 * rule inspects JSX text and string literals in expression containers, never an identifier.
 *
 * The addresses are **copied**, not opened. `mailto:` would mean widening the opener capability by
 * a whole URL scheme, and the scope's real job is bounding the two call sites that pass data rather
 * than constants (SECURITY §4). Clipboard-write is already granted, works with no mail client
 * installed, and the address has to be visible on screen either way.
 */

import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { REPO } from '@/components/legal'
import { useOpenExternal } from '@/components/useOpenExternal'
import { useAppInfo } from '@/screens/useAppInfo'
import { useLegalStore } from '@/stores'

/** SPDX identifier from `Cargo.toml` and `package.json`. An identifier, never translated. */
const LICENSE = 'GPL-3.0-only'

/** Where to write. Both are read. */
const CONTACT = {
  general: 'contact@poli0981.dev',
  code: 'code@poli0981.dev',
} as const

/** The one page listing every other channel. Scoped in `capabilities/external-links.json`. */
const LINKS = 'https://poli0981.dev/links/'

function CopyableAddress({ address, label }: { address: string; label: string }) {
  const { t } = useTranslation()
  const [copied, setCopied] = useState(false)

  return (
    <div className="mt-3">
      <p className="text-data text-text-dim">{label}</p>
      <p className="mt-1 flex flex-wrap items-center gap-2">
        <code className="font-mono text-data text-text">{address}</code>
        <button
          type="button"
          aria-label={t('about.copyAddress', { address })}
          onClick={() => {
            void writeText(address).then(() => {
              setCopied(true)
            })
          }}
          className="rounded-pd border border-border px-2 py-0.5 text-data text-text-dim hover:bg-surface-2"
        >
          {copied ? t('about.copied') : t('about.copy')}
        </button>
      </p>
    </div>
  )
}

export function PdAbout() {
  const { t } = useTranslation()
  const { open, failed: openFailed } = useOpenExternal()
  const openReview = useLegalStore((s) => s.openReview)
  const info = useAppInfo()

  return (
    <section aria-labelledby="about-title" className="h-full overflow-auto p-6">
      <h1 id="about-title" className="text-accent">
        {t('about.title')}
      </h1>

      <p className="mt-1 text-text-dim">{t('app.tagline')}</p>

      <p className="mt-6 max-w-2xl">{t('about.what')}</p>
      <p className="mt-3 max-w-2xl text-text-dim">{t('about.privacy')}</p>

      <h2 className="mt-8 text-text-dim">{t('about.buildTitle')}</h2>
      <dl className="mt-2 max-w-2xl space-y-2">
        <div className="flex flex-wrap gap-x-3">
          <dt className="w-40 shrink-0 text-data text-text-dim">{t('about.version')}</dt>
          {/* Nothing until it resolves: an em-dash here would be a state that was never loaded. */}
          <dd className="font-mono text-data text-text">{info === null ? null : info.version}</dd>
        </div>
        <div className="flex flex-wrap gap-x-3">
          <dt className="w-40 shrink-0 text-data text-text-dim">{t('about.license')}</dt>
          <dd className="font-mono text-data text-text">{LICENSE}</dd>
        </div>
        <div className="flex flex-wrap gap-x-3">
          <dt className="w-40 shrink-0 text-data text-text-dim">{t('about.docsHash')}</dt>
          <dd className="min-w-0">
            <code className="block break-all font-mono text-data text-text">
              {info === null ? null : info.docsHash}
            </code>
            <span className="mt-1 block text-data text-text-dim">{t('about.docsHashDetail')}</span>
          </dd>
        </div>
      </dl>

      <div className="mt-4 flex flex-wrap gap-2">
        <button
          type="button"
          onClick={() => {
            open(REPO)
          }}
          className="rounded-pd border border-border px-3 py-1 text-data text-text-dim hover:bg-surface-2"
        >
          {t('about.openRepo')}
        </button>
        <button
          type="button"
          onClick={openReview}
          className="rounded-pd border border-border px-3 py-1 text-data text-text-dim hover:bg-surface-2"
        >
          {t('about.reopen')}
        </button>
      </div>
      <p className="mt-2 max-w-2xl text-data text-text-dim">{t('about.reopenDetail')}</p>

      <h2 className="mt-8 text-text-dim">{t('about.contactTitle')}</h2>
      <p className="mt-1 max-w-2xl text-data text-text-dim">{t('about.contactIntro')}</p>
      <CopyableAddress address={CONTACT.general} label={t('about.contactGeneral')} />
      <CopyableAddress address={CONTACT.code} label={t('about.contactCode')} />

      <h2 className="mt-8 text-text-dim">{t('about.linksTitle')}</h2>
      <p className="mt-1 max-w-2xl text-data text-text-dim">{t('about.linksIntro')}</p>
      <p className="mt-2 flex flex-wrap items-center gap-2">
        <button
          type="button"
          onClick={() => {
            open(LINKS)
          }}
          className="rounded-pd border border-border px-3 py-1 text-data text-text-dim hover:bg-surface-2"
        >
          {t('about.openLinks')}
        </button>
        <code className="font-mono text-data text-text-dim">{LINKS}</code>
      </p>

      {openFailed ? (
        <p className="mt-4 text-data text-warn" role="alert">
          {t('actions.openFailed')}
        </p>
      ) : null}
    </section>
  )
}
