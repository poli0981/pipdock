/**
 * What a rollback would do — DATA-FLOW §8, rendered before it happens.
 *
 * The `unrestorable` list is the reason this component is not just a count. §8 says a release that
 * can no longer be fetched "cannot be restored → PD-SNP-002, listed explicitly; user may proceed
 * partially", and a preview that reported "12 packages" while silently leaving two behind would be
 * a success message for a restore that did not happen. So the list is shown, with its code, before
 * the confirm rather than in the summary afterwards.
 */

import { useTranslation } from 'react-i18next'

import type { RollbackPreview } from '@/ipc'

export function PdRollbackPreview({ preview }: { preview: RollbackPreview }) {
  const { t } = useTranslation()
  const { restore, unrestorable, target } = preview
  const empty = restore.uninstall.length === 0 && restore.install.length === 0

  return (
    <div>
      <p className="text-data text-text-dim">
        {t('snapshots.restoringTo')} <code className="font-mono">{target.id}</code>
      </p>

      {empty ? (
        <p className="mt-3 text-data text-text-dim">{t('snapshots.alreadyMatches')}</p>
      ) : null}

      {restore.uninstall.length > 0 ? (
        <section className="mt-4" data-section="uninstall">
          <h2 className="text-data text-warn">
            {t('snapshots.willRemove', { count: restore.uninstall.length })}
          </h2>
          <ul className="mt-1 space-y-0.5">
            {restore.uninstall.map((pkg) => (
              <li key={pkg}>
                <code className="font-mono text-data">{pkg}</code>
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      {restore.install.length > 0 ? (
        <section className="mt-4" data-section="install">
          <h2 className="text-data text-accent">
            {t('snapshots.willRestore', { count: restore.install.length })}
          </h2>
          <ul className="mt-1 space-y-0.5">
            {restore.install.map((spec) => (
              <li key={spec.name}>
                {/* Restored *at the snapshot's version*, which is the whole point — a bare name
                    would leave the reader unable to tell this from a plain reinstall. */}
                <code className="font-mono text-data">{`${spec.name}==${spec.version}`}</code>
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      {unrestorable.length > 0 ? (
        <section className="mt-4" data-section="unrestorable">
          <h2 className="text-data text-danger">
            <code className="font-mono">{'PD-SNP-002'}</code>{' '}
            {t('snapshots.unrestorable', { count: unrestorable.length })}
          </h2>
          <p className="mt-1 text-data text-text-dim">{t('snapshots.unrestorableDetail')}</p>
          <ul className="mt-1 space-y-0.5">
            {unrestorable.map((line) => (
              <li key={line}>
                {/* Verbatim freeze lines: an editable install or a direct URL, exactly as the
                    snapshot recorded it. Never reshaped (I18N §2). */}
                <code className="font-mono text-data text-text-dim">{line}</code>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
    </div>
  )
}
