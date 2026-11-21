/**
 * Fail when a package the production build can bundle is not under a GPL-3.0-compatible licence.
 *
 * The npm half of what `deny.toml` does for the crates. THIRD-PARTY-NOTICES says every bundled
 * licence is compatible with PipDock's GPL-3.0-only; `cargo deny` made that a test for the Rust
 * side in 1.4.0, and until this script SECURITY §7 had to admit that "the npm half of the build
 * has no equivalent licence check". Qodana's licence audit would have been one, but it needs the
 * Ultimate Plus plan, and a check this small should not depend on a plan.
 *
 * It reads `package-lock.json` (lockfile v3) and installs nothing. "Production" is every package
 * not marked `dev`: `devOptional` and `optional` ones stay in, because something the app depends
 * on can pull them in. The allowlist mirrors `deny.toml`'s, entry for entry; widening either is a
 * licensing decision, not a CI fix, and the two lists are meant to move together.
 */

import { readFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'

/** SPDX identifiers accepted on their own — exactly `deny.toml`'s `[licenses] allow`. */
export const ALLOWED = new Set([
  '0BSD',
  'Apache-2.0',
  'BSD-3-Clause',
  'BSL-1.0',
  'CC0-1.0',
  'ISC',
  'MIT',
  'MIT-0',
  'MPL-2.0',
  'Unicode-3.0',
  'Unlicense',
  'Zlib',
])

/**
 * Packages whose lockfile entry carries no usable SPDX expression, each with the licence a person
 * checked by hand and why. Empty: every production package declares one today.
 */
export const CHECKED_BY_HAND = new Map()

/** Split an SPDX expression into identifiers, operators and parentheses. */
export function tokenize(expression) {
  return expression
    .replace(/\(/g, ' ( ')
    .replace(/\)/g, ' ) ')
    .split(/\s+/)
    .filter(Boolean)
}

/**
 * Whether an SPDX expression can be satisfied with allowed licences: `OR` needs one side, `AND`
 * both, `AND` binds tighter than `OR`, and parentheses group. A `WITH` exception is accepted only
 * when the whole `id WITH exception` pair is allowed — none is today, so one fails and is looked
 * at. Anything malformed throws, and the caller reports it as a violation.
 */
export function satisfiable(expression, allowed = ALLOWED) {
  const tokens = tokenize(expression)
  let at = 0
  const peek = () => tokens[at]
  const take = () => tokens[at++]

  const primary = () => {
    const token = take()
    if (token === undefined) throw new Error(`unexpected end of "${expression}"`)
    if (token === '(') {
      const value = disjunction()
      if (take() !== ')') throw new Error(`unbalanced parenthesis in "${expression}"`)
      return value
    }
    if (token === ')' || token === 'AND' || token === 'OR' || token === 'WITH') {
      throw new Error(`unexpected "${token}" in "${expression}"`)
    }
    if (peek() === 'WITH') {
      take()
      const exception = take()
      if (exception === undefined) throw new Error(`"WITH" without an exception in "${expression}"`)
      return allowed.has(`${token} WITH ${exception}`)
    }
    return allowed.has(token)
  }
  const conjunction = () => {
    let value = primary()
    while (peek() === 'AND') {
      take()
      value = primary() && value
    }
    return value
  }
  const disjunction = () => {
    let value = conjunction()
    while (peek() === 'OR') {
      take()
      value = conjunction() || value
    }
    return value
  }

  const result = disjunction()
  if (at !== tokens.length) throw new Error(`trailing "${tokens[at]}" in "${expression}"`)
  return result
}

/** `node_modules/a/node_modules/@scope/b` → `@scope/b`. */
export function packageName(path) {
  return path.slice(path.lastIndexOf('node_modules/') + 'node_modules/'.length)
}

/** Every non-dev package in a lockfile-v3 `packages` map, as `{ name, version, license }`. */
export function productionPackages(lock) {
  return Object.entries(lock.packages ?? {})
    .filter(([path, entry]) => path !== '' && !entry.dev && !entry.link)
    .map(([path, entry]) => ({ name: packageName(path), version: entry.version, license: entry.license }))
}

/** The packages that fail the allowlist, each with the reason. */
export function violations(packages, allowed = ALLOWED, checkedByHand = CHECKED_BY_HAND) {
  const found = []
  for (const pkg of packages) {
    if (checkedByHand.has(pkg.name)) continue
    if (typeof pkg.license !== 'string' || pkg.license.trim() === '') {
      found.push({ ...pkg, reason: 'declares no SPDX licence expression' })
      continue
    }
    try {
      if (!satisfiable(pkg.license, allowed)) found.push({ ...pkg, reason: `${pkg.license} is not allowed` })
    } catch (error) {
      found.push({ ...pkg, reason: error.message })
    }
  }
  return found
}

function main() {
  const lock = JSON.parse(readFileSync('package-lock.json', 'utf8'))
  if (lock.lockfileVersion !== 3) {
    console.error(`package-lock.json is lockfile v${lock.lockfileVersion}; this check reads v3`)
    return 1
  }
  const packages = productionPackages(lock)
  const found = violations(packages)
  for (const v of found) console.error(`${v.name}@${v.version}: ${v.reason}`)
  if (found.length > 0) {
    console.error(
      `${found.length} of ${packages.length} production packages fail the licence allowlist. ` +
        'Widening it is a licensing decision (deny.toml says why); CHECKED_BY_HAND is for a ' +
        'package whose metadata is missing or wrong, with the licence a person verified.',
    )
    return 1
  }
  console.log(`${packages.length} production packages, every one under a GPL-3.0-compatible licence.`)
  return 0
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  process.exitCode = main()
}
