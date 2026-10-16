import { describe, expect, it } from 'vitest'

import { isBrowserShortcut } from '@/components/browserKeys'

const press = (key: string, code: string, mods: Partial<Record<'ctrl' | 'shift' | 'alt', boolean>> = {}) =>
  isBrowserShortcut({
    key,
    code,
    ctrlKey: mods.ctrl === true,
    shiftKey: mods.shift === true,
    altKey: mods.alt === true,
    metaKey: false,
  })

describe('the browser shortcuts a release build swallows', () => {
  it.each([
    ['F5', 'F5', {}],
    ['r', 'KeyR', { ctrl: true }],
    ['R', 'KeyR', { ctrl: true, shift: true }],
    ['p', 'KeyP', { ctrl: true }],
    ['f', 'KeyF', { ctrl: true }],
    ['F3', 'F3', {}],
    ['ArrowLeft', 'ArrowLeft', { alt: true }],
    ['BrowserBack', 'BrowserBack', {}],
  ] as const)('%s (%s) is the browser’s', (key, code, mods) => {
    expect(press(key, code, mods)).toBe(true)
  })

  it('finds Ctrl+R by its physical key, whatever the layout calls it', () => {
    // A layout where KeyR types something else still reloads the page.
    expect(press('к', 'KeyR', { ctrl: true })).toBe(true)
  })

  it.each([
    ['k', 'KeyK', { ctrl: true }],
    ['a', 'KeyA', { ctrl: true }],
    ['c', 'KeyC', { ctrl: true }],
    ['v', 'KeyV', { ctrl: true }],
    ['1', 'Digit1', { ctrl: true }],
    ['/', 'Slash', {}],
    ['r', 'KeyR', {}],
    ['ArrowLeft', 'ArrowLeft', {}],
    ['r', 'KeyR', { ctrl: true, alt: true }],
  ] as const)('%s (%s) is left to the app', (key, code, mods) => {
    // Ctrl+K, Ctrl+A, Ctrl+1..9 and `/` are PipDock's own (UI-SPEC §8); copy and paste are the
    // user's; a bare R is typing; Ctrl+Alt is AltGr on many layouts.
    expect(press(key, code, mods)).toBe(false)
  })
})
