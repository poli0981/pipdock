import js from '@eslint/js'
import tseslint from 'typescript-eslint'
import reactHooks from 'eslint-plugin-react-hooks'
import reactRefresh from 'eslint-plugin-react-refresh'

/**
 * `docs/I18N.md` §1 — the no-hardcoded-strings rule.
 *
 * Every user-visible string must come from an i18next catalog, so untranslated copy can never
 * ship. The documented allowlist is `PipDock`, package names, versions and catalog codes, which is
 * pattern-shaped rather than a fixed word list — so this is a local rule instead of
 * `react/jsx-no-literals` (which also caps out at ESLint 9 and pulls a vulnerable minimatch).
 */
const noJsxLiterals = {
  meta: {
    type: 'problem',
    docs: { description: 'JSX text must come from an i18next catalog (docs/I18N.md §1)' },
    schema: [],
    messages: {
      literal:
        'Hardcoded JSX text {{text}} — use t() with a key from ui/src/locales (docs/I18N.md §1).',
    },
  },
  create(context) {
    // Allowed without translation: the product name, PEP 503 package names, PEP 440 versions,
    // catalog codes, and pure punctuation/symbols (the terminal glyphs in UI-SPEC §7).
    const allowed = [
      /^PipDock$/,
      /^[a-z0-9][a-z0-9._-]*$/, // package name
      /^v?\d+(\.\d+)*([a-z].*)?$/i, // version
      /^PD-[A-Z]{3}-\d{3}$/, // catalog code
      /^[^\p{L}\p{N}]+$/u, // punctuation and glyphs only
    ]
    const check = (node, raw) => {
      const text = raw.trim()
      if (!text) return
      if (allowed.some((re) => re.test(text))) return
      context.report({ node, messageId: 'literal', data: { text: JSON.stringify(text) } })
    }
    return {
      JSXText: (node) => check(node, node.value),
      JSXExpressionContainer(node) {
        if (node.expression.type === 'Literal' && typeof node.expression.value === 'string') {
          check(node, node.expression.value)
        }
      },
    }
  },
}

export default tseslint.config(
  { ignores: ['dist/**', 'target/**', 'src-tauri/target/**', 'node_modules/**'] },

  js.configs.recommended,
  ...tseslint.configs.recommendedTypeChecked,

  {
    files: ['ui/src/**/*.{ts,tsx}'],
    languageOptions: {
      parserOptions: { projectService: true, tsconfigRootDir: import.meta.dirname },
    },
    plugins: {
      'react-hooks': reactHooks,
      'react-refresh': reactRefresh,
      pipdock: { rules: { 'no-jsx-literals': noJsxLiterals } },
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      'react-refresh/only-export-components': ['warn', { allowConstantExport: true }],
      'pipdock/no-jsx-literals': 'error',

      // ARCHITECTURE §9: no component calls `invoke` directly — everything goes through the typed
      // wrappers in ui/src/ipc, which are the only place Rust types cross into TS.
      'no-restricted-imports': [
        'error',
        {
          paths: [
            {
              name: '@tauri-apps/api/core',
              importNames: ['invoke'],
              message: 'Import a typed wrapper from @/ipc instead (ARCHITECTURE §9).',
            },
          ],
        },
      ],
    },
  },

  // The IPC layer is the one place allowed to call invoke().
  {
    files: ['ui/src/ipc/**/*.ts'],
    rules: { 'no-restricted-imports': 'off' },
  },

  // Tests assert on real copy, so literals are expected there.
  {
    files: ['ui/src/**/*.test.{ts,tsx}', 'ui/src/test/**/*.{ts,tsx}'],
    rules: { 'pipdock/no-jsx-literals': 'off' },
  },

  // Config files are plain Node modules, outside the type-aware program.
  {
    files: ['*.js', '*.ts'],
    ...tseslint.configs.disableTypeChecked,
  },
)
