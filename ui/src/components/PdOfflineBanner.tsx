/**
 * The offline chip — UI-SPEC §7: "banner chip in top bar; search works (local index), metadata
 * panel shows cached-at timestamp".
 *
 * Deliberately **not** a blocking state. Search is served from a local index and keeps working;
 * only the things that genuinely need the network stop. A banner that greyed the app out would be
 * claiming more than is true.
 *
 * `navigator.onLine` is the only signal available in a webview, and it is a weak one — it reports
 * whether an interface is up, not whether PyPI is reachable. So it is treated as a hint that
 * explains a failure the user is already seeing, never as a gate on trying.
 */

import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

/** True when the OS says there is no connection. Not exported: the banner is its only reader,
 * and exporting it from a component file breaks fast refresh. */
function useOnline(): boolean {
  const [online, setOnline] = useState(() => navigator.onLine)

  useEffect(() => {
    const up = () => {
      setOnline(true)
    }
    const down = () => {
      setOnline(false)
    }
    window.addEventListener('online', up)
    window.addEventListener('offline', down)
    return () => {
      window.removeEventListener('online', up)
      window.removeEventListener('offline', down)
    }
  }, [])

  return online
}

export function PdOfflineBanner() {
  const { t } = useTranslation()
  const online = useOnline()

  if (online) return null

  return (
    <span
      role="status"
      title={t('search.offlineDetail')}
      className="rounded-pd bg-warn/20 px-2 py-0.5 text-data text-warn"
    >
      {t('search.offline')}
    </span>
  )
}
