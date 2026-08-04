import '@testing-library/jest-dom/vitest'

// The real i18next instance with the real catalogs, exactly as `main.tsx` initializes it.
// Component tests then assert against shipped copy rather than raw keys, so a key that was never
// added to the catalog fails the test that uses it instead of silently rendering its own name.
import '@/i18n'

/**
 * Give jsdom a viewport.
 *
 * jsdom reports every element as 0×0 and has no ResizeObserver. A virtualizer asked how tall its
 * scroll element is therefore hears "zero", renders **no rows at all**, and every assertion about
 * row contents passes because there is nothing to contradict it — a green suite testing nothing.
 *
 * `PdPackageTable` also takes an `initialRect` prop for the same reason; these stubs cover the
 * measurements the virtualizer takes afterwards. The windowing assertion in
 * `PdPackageTable.test.tsx` is what proves both are working.
 */
const VIEWPORT = { width: 1280, height: 600 }

globalThis.ResizeObserver ??= class {
  observe() {
    /* the size never changes under test */
  }
  unobserve() {
    /* no-op */
  }
  disconnect() {
    /* no-op */
  }
}

Object.defineProperty(HTMLElement.prototype, 'offsetHeight', {
  configurable: true,
  get() {
    return VIEWPORT.height
  },
})

Object.defineProperty(HTMLElement.prototype, 'offsetWidth', {
  configurable: true,
  get() {
    return VIEWPORT.width
  },
})

Element.prototype.getBoundingClientRect = function getBoundingClientRect(): DOMRect {
  return {
    x: 0,
    y: 0,
    top: 0,
    left: 0,
    right: VIEWPORT.width,
    bottom: VIEWPORT.height,
    width: VIEWPORT.width,
    height: VIEWPORT.height,
    toJSON: () => ({}),
  }
}

export { VIEWPORT }
