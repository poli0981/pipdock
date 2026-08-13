/**
 * Build `pipdock.exe` and stage it as a Tauri sidecar, so the installer ships the CLI.
 *
 * **Why this exists.** The first bundle built for Phase 4 contained exactly one binary,
 * `pipdock-app.exe`. PRD P0-12 is "CLI core parity", CLI-SPEC documents the whole surface, and the
 * README's Quick start tells the user to run `pipdock env list` — none of which was on the machine
 * after a real install. Found by listing the installer, not by reading the config.
 *
 * Tauri's `externalBin` wants the host target triple in the filename and strips it when bundling,
 * so the installed name is plain `pipdock.exe`, next to the GUI. **`PATH` is deliberately not
 * touched** (owner decision, 2026-08-13): PipDock does not modify the user's environment, which is
 * the same restraint SECURITY §5 applies to not shipping a self-updater. The README says where the
 * binary lands and how to put it on `PATH` if you want it there.
 *
 * Run from `beforeBuildCommand` rather than only from the release workflow, so a local
 * `tauri build` and CI produce the same installer — a bundle that differs by who built it is how
 * "works on my machine" gets into a release.
 */

import { execFileSync } from 'node:child_process'
import { copyFileSync, mkdirSync } from 'node:fs'
import { join } from 'node:path'

/** The host triple, asked of rustc rather than assumed — cross-compiling would break a guess. */
function hostTriple() {
  const out = execFileSync('rustc', ['-vV'], { encoding: 'utf8' })
  const line = out.split('\n').find((l) => l.startsWith('host:'))
  if (line === undefined) throw new Error('rustc -vV printed no host line')
  return line.slice('host:'.length).trim()
}

const triple = hostTriple()
const target = join('src-tauri', 'binaries')
mkdirSync(target, { recursive: true })

// `--locked` for the reason the release workflow passes it: a bundle is not the place to discover
// that a dependency resolved differently than the lockfile says.
execFileSync('cargo', ['build', '--release', '--locked', '-p', 'pipdock-cli'], {
  stdio: 'inherit',
})

const from = join('target', 'release', 'pipdock.exe')
const to = join(target, `pipdock-${triple}.exe`)
copyFileSync(from, to)
console.log(`staged ${from} -> ${to}`)
