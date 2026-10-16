/**
 * The one test for "is this a `PdError`", in a module with no Tauri import.
 *
 * Both stores route every rejection through it, so the `@/ipc` mock has to use the same predicate
 * rather than a lookalike — it used one, which accepted `{ code: 1, message: null }`, and every
 * error test agreed with itself (U10). Living here lets the mock import the real thing without
 * importing `@tauri-apps/api`, which has no host to talk to under jsdom.
 */

import type { PdError } from '@/ipc'

export function isPdError(value: unknown): value is PdError {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as PdError).code === 'string' &&
    typeof (value as PdError).message === 'string'
  )
}
