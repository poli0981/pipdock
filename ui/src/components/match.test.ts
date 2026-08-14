/**
 * The palette's matcher.
 *
 * The ranking rules are SP-3's, learned from nucleo putting `requests-ntlm` above `requests`: a
 * raw fuzzy score rewards density, so tiers come first and length breaks ties. These cases are
 * that finding restated over action labels.
 */

import { describe, expect, it } from 'vitest'

import { isSubsequence, rank } from '@/components/match'

const label = (s: string) => [s] as const

describe('isSubsequence', () => {
  it('requires order, not merely presence', () => {
    // The whole reason this is a two-pointer walk. `[...needle].every(c => hay.includes(c))` is
    // the obvious wrong implementation and would say true here.
    expect(isSubsequence('slot', 'tools')).toBe(false)
    expect(isSubsequence('tls', 'tools')).toBe(true)
  })

  it('matches a prefix, a scattered run and the whole string', () => {
    expect(isSubsequence('too', 'tools')).toBe(true)
    expect(isSubsequence('tos', 'tools')).toBe(true)
    expect(isSubsequence('tools', 'tools')).toBe(true)
    expect(isSubsequence('toolss', 'tools')).toBe(false)
  })

  it('is true for an empty needle', () => {
    expect(isSubsequence('', 'anything')).toBe(true)
  })
})

describe('rank', () => {
  it('keeps everything, in order, for an empty query', () => {
    // A palette that opens showing nothing has to be taught before it can be used.
    const items = ['Updates', 'Search', 'Pins']
    expect(rank(items, '   ', label)).toEqual(items)
  })

  it('puts the exact match first, then the prefix, then the subsequence', () => {
    const got = rank(['Pins and things', 'Pin', 'Pins', 'Preferences in'], 'pin', label)
    expect(got.slice(0, 3)).toEqual(['Pin', 'Pins', 'Pins and things'])
  })

  it('prefers the shorter label within a tier', () => {
    // SP-3's finding in miniature: the short thing containing the query is nearly always the
    // thing itself, and the longer ones are its variants.
    expect(rank(['Search settings', 'Search'], 'search', label)).toEqual([
      'Search',
      'Search settings',
    ])
  })

  it('is alphabetical on a full tie, so two identical queries never reorder', () => {
    expect(rank(['beta', 'alpha'], 'a', label)).toEqual(['alpha', 'beta'])
  })

  it('drops what does not match at all', () => {
    expect(rank(['Updates', 'Health'], 'zzz', label)).toEqual([])
  })

  it('is case-insensitive in both directions', () => {
    expect(rank(['Health'], 'HEALTH', label)).toEqual(['Health'])
    expect(rank(['HEALTH'], 'health', label)).toEqual(['HEALTH'])
  })

  it('puts every label match above every group match', () => {
    // Found by a failing palette test. With one flat `${group} ${label}` string, typing `search`
    // ranked *Download the index* — group "Search", a prefix match — above the Search tab, whose
    // label matched only as a subsequence of `go to search`. Someone typing a screen's name wants
    // the screen, however weakly the label matched.
    const items = [
      { label: 'Download the index', group: 'Search' },
      { label: 'Search', group: 'Go to' },
    ]
    const got = rank(items, 'search', (i) => [i.label, i.group])
    expect(got[0]?.label).toBe('Search')
  })

  it('still finds an item by its group when no label matches', () => {
    const items = [{ label: 'Download the index', group: 'Search' }]
    expect(rank(items, 'search', (i) => [i.label, i.group])).toEqual(items)
  })
})
