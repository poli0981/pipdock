// Run with `node --test scripts/*.test.mjs` — Node's built-in runner, so the check needs no
// dependency of its own (ci-node.yml's `npm licences` job).

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { packageName, productionPackages, satisfiable, violations } from './check-licenses.mjs'

test('a single allowed identifier passes, an unknown one does not', () => {
  assert.equal(satisfiable('MIT'), true)
  assert.equal(satisfiable('CC-BY-4.0'), false)
  assert.equal(satisfiable('GPL-2.0-only'), false)
})

test('OR needs one allowed side, AND needs both', () => {
  assert.equal(satisfiable('MIT OR Apache-2.0'), true)
  assert.equal(satisfiable('(GPL-2.0-only OR MIT)'), true)
  assert.equal(satisfiable('MIT AND ISC'), true)
  assert.equal(satisfiable('MIT AND CC-BY-4.0'), false)
})

test('AND binds tighter than OR, and parentheses override it', () => {
  // MIT OR (CC-BY-4.0 AND ISC) — the MIT branch carries it.
  assert.equal(satisfiable('MIT OR CC-BY-4.0 AND ISC'), true)
  // (MIT OR CC-BY-4.0) AND WTFPL — WTFPL is not on the list, so nothing can satisfy it.
  assert.equal(satisfiable('(MIT OR CC-BY-4.0) AND WTFPL'), false)
  // An operand read left of OR must not be dropped when the right side is false.
  assert.equal(satisfiable('Apache-2.0 OR CC-BY-4.0'), true)
})

test('an exception is accepted only as a whole pair someone allowed', () => {
  assert.equal(satisfiable('Apache-2.0 WITH LLVM-exception'), false)
  const allowed = new Set(['Apache-2.0 WITH LLVM-exception'])
  assert.equal(satisfiable('Apache-2.0 WITH LLVM-exception', allowed), true)
})

test('a malformed expression throws rather than passing', () => {
  assert.throws(() => satisfiable('(MIT'), /unbalanced|end/)
  assert.throws(() => satisfiable('MIT OR'), /end/)
  assert.throws(() => satisfiable('MIT ISC'), /trailing/)
  assert.throws(() => satisfiable('AND MIT'), /unexpected/)
})

test('package names come from the last node_modules segment, scopes included', () => {
  assert.equal(packageName('node_modules/react'), 'react')
  assert.equal(packageName('node_modules/@tauri-apps/api'), '@tauri-apps/api')
  assert.equal(packageName('node_modules/a/node_modules/@scope/b'), '@scope/b')
})

test('production means everything not marked dev, optional and devOptional included', () => {
  const lock = {
    packages: {
      '': { name: 'pipdock', version: '1.0.0' },
      'node_modules/react': { version: '19.0.0', license: 'MIT' },
      'node_modules/vitest': { version: '5.0.0', license: 'MIT', dev: true },
      'node_modules/fsevents': { version: '2.3.3', license: 'MIT', optional: true },
      'node_modules/lightningcss': { version: '1.0.0', license: 'MPL-2.0', devOptional: true },
      'node_modules/linked': { link: true, resolved: 'ui' },
    },
  }
  assert.deepEqual(
    productionPackages(lock).map((p) => p.name),
    ['react', 'fsevents', 'lightningcss'],
  )
})

test('violations name the package and the reason, and a hand-checked one is skipped', () => {
  const packages = [
    { name: 'ok', version: '1.0.0', license: 'MIT' },
    { name: 'data', version: '1.0.0', license: 'CC-BY-4.0' },
    { name: 'silent', version: '1.0.0', license: undefined },
    { name: 'broken', version: '1.0.0', license: '(MIT' },
  ]
  const found = violations(packages)
  assert.deepEqual(
    found.map((v) => v.name),
    ['data', 'silent', 'broken'],
  )
  assert.match(found[1].reason, /no SPDX/)
  const handChecked = new Map([['silent', 'MIT — LICENSE file read by hand']])
  assert.deepEqual(
    violations(packages, undefined, handChecked).map((v) => v.name),
    ['data', 'broken'],
  )
})
