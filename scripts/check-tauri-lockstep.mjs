/**
 * Fail when the Tauri crates and the `@tauri-apps/*` packages are on different minors.
 *
 * Tauri checks this itself and refuses to start — at runtime, on the user's machine — when the
 * `tauri` crate and `@tauri-apps/api` disagree on major.minor. Dependabot bumps Cargo and npm in
 * separate pull requests, each green on its own, so merging one of them alone ships a build that
 * compiles and does not open. This reads both lockfiles, so it checks what is built rather than
 * what the manifests ask for.
 *
 * Plugins move independently of the core (`tauri-plugin-dialog` 2.8 sits beside `tauri` 2.12), so
 * each plugin crate is compared with its own npm package, not with `tauri`.
 */

import { readFileSync } from 'node:fs'

const PAIRS = [
  ['tauri', '@tauri-apps/api'],
  ['tauri-plugin-dialog', '@tauri-apps/plugin-dialog'],
  ['tauri-plugin-opener', '@tauri-apps/plugin-opener'],
  ['tauri-plugin-clipboard-manager', '@tauri-apps/plugin-clipboard-manager'],
]

/** Every `name = "…" / version = "…"` pair in Cargo.lock. */
function crateVersions() {
  const lock = readFileSync('Cargo.lock', 'utf8')
  const out = new Map()
  for (const block of lock.split('[[package]]')) {
    const name = /^name = "([^"]+)"$/m.exec(block)?.[1]
    const version = /^version = "([^"]+)"$/m.exec(block)?.[1]
    if (name !== undefined && version !== undefined) out.set(name, version)
  }
  return out
}

function npmVersion(name) {
  const lock = JSON.parse(readFileSync('package-lock.json', 'utf8'))
  return lock.packages?.[`node_modules/${name}`]?.version
}

const minor = (v) => v.split('.').slice(0, 2).join('.')

const crates = crateVersions()
let failed = false
for (const [crate, pkg] of PAIRS) {
  const rust = crates.get(crate)
  const js = npmVersion(pkg)
  if (rust === undefined || js === undefined) {
    console.error(`missing: ${crate} ${rust ?? '(not in Cargo.lock)'} / ${pkg} ${js ?? '(not in package-lock.json)'}`)
    failed = true
    continue
  }
  const ok = minor(rust) === minor(js)
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${crate} ${rust}  ${pkg} ${js}`)
  if (!ok) failed = true
}
if (failed) {
  console.error('Bump the crate and its npm package together; Tauri will not start otherwise.')
  process.exit(1)
}
