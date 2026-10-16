/**
 * The browser's own shortcuts, which a release build has no use for — U20.
 *
 * WebView2 is a browser, and it kept a browser's keys: F5 and Ctrl+R reloaded the app — dropping a
 * preview mid-decision along with everything the stores had not persisted — Ctrl+P printed the
 * package table, Ctrl+F opened a find bar over a virtualized table whose off-screen rows do not
 * exist in the DOM, and Alt+← navigated "back" from a single-page app. None of them is a PipDock
 * feature, and each looked like one.
 *
 * Development keeps them: a reload is how a developer recovers from a bad hot update.
 */

type Keys = Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey' | 'shiftKey' | 'altKey' | 'metaKey'>

/** Physical keys whose Ctrl chord is the browser's: reload, print, find, find-next, source, save. */
const CTRL_CODES: ReadonlySet<string> = new Set(['KeyR', 'KeyP', 'KeyF', 'KeyG', 'KeyU', 'KeyS'])

/** Keys that are the browser's on their own. */
const BARE_KEYS: ReadonlySet<string> = new Set([
  'F3',
  'F5',
  'BrowserBack',
  'BrowserForward',
  'BrowserRefresh',
])

export function isBrowserShortcut(e: Keys): boolean {
  if (BARE_KEYS.has(e.key)) return true
  if (e.altKey && !e.ctrlKey && !e.metaKey && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) {
    return true
  }
  if (!e.ctrlKey || e.altKey || e.metaKey) return false
  // By physical key, like the app's own shortcuts: on AZERTY, Ctrl+R is still `KeyR`.
  return CTRL_CODES.has(e.code)
}
