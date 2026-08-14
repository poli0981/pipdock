/**
 * Export and import a `requirements.txt` — PRD P1-3.
 *
 * On the environment detail screen, beside snapshots, because both are *the environment as a
 * document*: one is PipDock's own record and one is the file the rest of the Python world passes
 * around. The exported bytes are the engine's `freeze`, which is what a snapshot stores — so there
 * is one idea of what an environment written down looks like, not two.
 *
 * **Import does not install.** It reads, reports, and hands the specs to the ordinary install flow
 * — which previews, snapshots and confirms like every other mutation. The skipped lines are shown
 * *before* that, because a file asking for `-r dev.txt` or an editable install is asking for
 * something PipDock will not do, and a preview that quietly represented a shorter list would be
 * accurate about itself and wrong about the user's file.
 */

import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { PdErrorRow } from '@/components/PdErrorRow'
import {
  envExport,
  pickOpenFile,
  pickSavePath,
  requirementsRead,
  type ParsedRequirements,
  type PdError,
  type PyEnv,
} from '@/ipc'
import { asPdError, usePlanStore } from '@/stores'

/** What the save dialog suggests. A filename, not copy — never translated (I18N §2). */
const DEFAULT_NAME = 'requirements.txt'

export function PdRequirements({ env }: { env?: PyEnv | undefined }) {
  const { t } = useTranslation()
  const resolve = usePlanStore((s) => s.resolve)
  const [busy, setBusy] = useState(false)
  const [wrote, setWrote] = useState<string | null>(null)
  const [read, setRead] = useState<ParsedRequirements | null>(null)
  const [error, setError] = useState<PdError | null>(null)

  const usable = env !== undefined

  const doExport = async () => {
    setError(null)
    setWrote(null)
    const path = await pickSavePath(t('requirements.exportTitle'), DEFAULT_NAME)
    // A cancelled picker is not a failure and must not read as one.
    if (path === null || env === undefined) return
    setBusy(true)
    try {
      setWrote(await envExport(env, path))
    } catch (e) {
      setError(asPdError(e))
    } finally {
      setBusy(false)
    }
  }

  const doRead = async () => {
    setError(null)
    setWrote(null)
    setRead(null)
    const path = await pickOpenFile(t('requirements.importTitle'), ['txt'])
    if (path === null) return
    setBusy(true)
    try {
      setRead(await requirementsRead(path))
    } catch (e) {
      setError(asPdError(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <section aria-labelledby="requirements-title" className="mt-4">
      <h2 id="requirements-title" className="text-text-dim">
        {t('requirements.title')}
      </h2>
      <p className="mt-1 max-w-2xl text-data text-text-dim">{t('requirements.intro')}</p>

      <div className="mt-2 flex flex-wrap gap-2">
        <button
          type="button"
          disabled={!usable || busy}
          onClick={() => {
            void doExport()
          }}
          data-action="export-requirements"
          className="rounded-pd border border-border px-3 py-1 text-data disabled:opacity-40"
        >
          {t('requirements.export')}
        </button>
        <button
          type="button"
          disabled={!usable || busy}
          onClick={() => {
            void doRead()
          }}
          data-action="import-requirements"
          className="rounded-pd border border-border px-3 py-1 text-data disabled:opacity-40"
        >
          {t('requirements.import')}
        </button>
      </div>

      {error !== null ? (
        <div className="mt-2">
          <PdErrorRow error={error} />
        </div>
      ) : null}

      {wrote !== null ? (
        // The path, so the user knows where it went rather than trusting that it went somewhere.
        <p className="mt-2 text-data text-text-dim">
          {t('requirements.wrote')} <code className="font-mono break-all">{wrote}</code>
        </p>
      ) : null}

      {read !== null ? (
        <div className="mt-3 rounded-pd border border-border bg-surface p-3">
          <p className="text-data">
            {t('requirements.readCount', { count: read.specs.length })}
          </p>

          {read.skipped.length > 0 ? (
            <>
              {/* Before the install button, not after it. These lines are the file asking for
                  something PipDock will not do, and the decision to proceed anyway is only
                  informed if it is made after reading them. */}
              <p className="mt-2 text-data text-warn">
                {t('requirements.skippedCount', { count: read.skipped.length })}
              </p>
              <ul className="mt-1 space-y-0.5">
                {read.skipped.map((s) => (
                  <li key={`${String(s.line)}:${s.text}`} className="text-data text-text-dim">
                    <code className="font-mono">{`${String(s.line)}:`}</code>{' '}
                    <code className="font-mono break-all">{s.text}</code>{' '}
                    <span>{t(`requirements.skip.${s.reason}`)}</span>
                  </li>
                ))}
              </ul>
            </>
          ) : null}

          <button
            type="button"
            disabled={!usable || read.specs.length === 0}
            onClick={() => {
              if (env === undefined) return
              // The ordinary install flow: preview, snapshot, confirm. Nothing about arriving
              // from a file earns an exemption from DATA-FLOW §9.
              void resolve(env, { intent: 'install', specs: read.specs })
            }}
            data-action="install-requirements"
            className="mt-3 rounded-pd bg-accent px-3 py-1 text-data text-bg disabled:opacity-40"
          >
            {t('requirements.install', { count: read.specs.length })}
          </button>
        </div>
      ) : null}
    </section>
  )
}
