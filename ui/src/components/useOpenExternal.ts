/**
 * Open a URL in the user's browser, and know when it did not.
 *
 * Every external link goes through Rust: `connect-src` in `tauri.conf.json` allows only `'self'`
 * and the IPC origin, so the webview cannot navigate anywhere. `opener:allow-open-url` is then
 * **scoped** (`capabilities/external-links.json`), and a URL outside that scope rejects with
 * `ForbiddenUrl`.
 *
 * That rejection is why this exists. `void openUrl(…)` — which four call sites used — discards it:
 * the promise rejects, nothing opens, and nothing anywhere says so. SECURITY §4 names that silence
 * as the reason an allowlist widening is recorded in prose rather than only in `capabilities/`;
 * this is the same fact from the UI side. `PdLegalGate` already handled it by hand, because a
 * document the user is being asked to agree to must never fail to appear. The rest now do too.
 *
 * `failed` latches: it stays true until the next successful open. A link that fails once has a
 * cause the user needs to see, and a message that cleared itself on the next render would be a
 * message nobody read.
 */

import { useCallback, useState } from 'react'

import { openUrl } from '@tauri-apps/plugin-opener'

export function useOpenExternal(): { open: (href: string) => void; failed: boolean } {
  const [failed, setFailed] = useState(false)

  const open = useCallback((href: string) => {
    openUrl(href).then(
      () => {
        setFailed(false)
      },
      () => {
        setFailed(true)
      },
    )
  }, [])

  return { open, failed }
}
