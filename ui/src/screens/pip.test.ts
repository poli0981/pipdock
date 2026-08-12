import { describe, expect, it } from 'vitest'

import { PIP_MIN_FOR_REPORT, pipNeedsUpgrade } from './pip'

describe('when the row offers to upgrade pip', () => {
  it('offers below the planner floor', () => {
    for (const version of ['21.3.1', '22.1', '22.1.2', '9.0.1', '20.0']) {
      expect(pipNeedsUpgrade(version), version).toBe(true)
    }
  })

  it('stays out of the way at or above it', () => {
    for (const version of ['22.2', '22.2.1', '23.0', '25.0.1', '26.2.1']) {
      expect(pipNeedsUpgrade(version), version).toBe(false)
    }
  })

  it('does not offer when the probe found no pip', () => {
    // A --without-pip venv, or -I hiding a user-site install. Offering here would run a command
    // that fails, so absence must not read as "old".
    expect(pipNeedsUpgrade(undefined)).toBe(false)
  })

  it('does not offer on a version it cannot read', () => {
    // Versions are data from the environment. Guessing wrong puts a mutating button on a row that
    // did not need one, so anything unparseable declines.
    for (const version of ['', 'unknown', 'v22', 'twenty-two']) {
      expect(pipNeedsUpgrade(version), JSON.stringify(version)).toBe(false)
    }
  })

  it('reads the release segments and ignores any suffix', () => {
    // pip ships pre-releases; `22.1b1` is still below the floor and `22.3.dev0` is still above it.
    expect(pipNeedsUpgrade('22.1b1')).toBe(true)
    expect(pipNeedsUpgrade('22.3.dev0')).toBe(false)
  })

  it('compares minors numerically, not as text', () => {
    // The bug this exists to prevent: "22.10" sorts before "22.2" as a string, so a lexical
    // comparison would offer to upgrade a pip that is eight releases newer than the floor.
    expect(pipNeedsUpgrade('22.10')).toBe(false)
  })

  it('mirrors the floor DATA-FLOW §7 documents', () => {
    expect(PIP_MIN_FOR_REPORT).toEqual([22, 2])
  })
})
