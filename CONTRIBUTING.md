# Contributing to PipDock

Thank you for wanting to help. PipDock is made by one person, so the most useful contributions are
the ones that arrive easy to say yes to: a bug report that reproduces, or a pull request that does
one thing and proves it.

By taking part you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).

## Before you write code

- **A bug** — [open an issue](https://github.com/poli0981/pipdock/issues/new/choose). The in-app
  *Report bug* button fills most of the form in for you.
- **Something that gets in your way as a keyboard, screen-reader or high-contrast user** — see
  [ACCESSIBILITY.md](ACCESSIBILITY.md). That report does not need a diagnosis.
- **A security problem** — do not open an issue; follow the [security policy](.github/SECURITY.md).
- **A question or an idea** — [Discussions](https://github.com/poli0981/pipdock/discussions).
- **A feature** — please ask first. The design is written down in `docs/`, the feature list and its
  non-goals are in [`docs/PRD.md`](docs/PRD.md), and what is planned and why is in
  [`docs/ROADMAP.md`](docs/ROADMAP.md). A pull request that contradicts those will be asked to
  change them first, and that is a conversation better had before the code.

## Setting up

PipDock is a Windows application: a Rust core, a Tauri 2 shell and a React 19 frontend.

| You need | Version |
|---|---|
| Windows | 10 or 11, with the WebView2 runtime (Windows 11 has it) |
| Rust | the toolchain in `rust-toolchain.toml`; `rustup` installs it on first use |
| Node.js | the major in `.nvmrc` |
| Python | 3.10 or newer, for the tests that drive a real pip |
| Build tools | the MSVC C++ build tools, as [Tauri requires](https://tauri.app/start/prerequisites/) |

```bash
npm ci
```

```bash
npm run tauri dev
```

`package.json` sits at the repository root although the frontend lives in `ui/`; that is deliberate
and Vite is configured for it.

## The checks, which are the ones CI runs

```bash
npm run lint
```

```bash
npm run typecheck
```

```bash
npm test
```

```bash
cargo fmt --all -- --check
```

```bash
cargo clippy --workspace --all-targets --locked -- -D warnings
```

```bash
cargo test --workspace --locked
```

A pull request needs five checks green before it can merge — `fmt · clippy · test`, `cargo audit`,
`lint · typecheck · test · build`, `npm audit` and CodeQL for JavaScript/TypeScript. Several more
run and are read before merging (`cargo deny`, the Tauri version lockstep, CodeQL for Rust and
Actions, the real-engine integration tests, SonarQube Cloud). New code is expected to be covered by
tests; the SonarQube gate asks for 80 % on new lines.

## What a pull request should look like

**One pull request, one thing, taken all the way.** Not several shallow changes. The
[template](.github/PULL_REQUEST_TEMPLATE.md) is a checklist of the rest; the parts that surprise
people are below.

- **A test that fails first.** Write the test, watch it fail for the reason you expect, then make
  the change. A test that has never failed has not been shown to test anything.
- **Run it.** Nearly every unplanned defect in this project was found by running the application or
  the CLI against a real environment, not by a passing suite. Say in the pull request what you ran.
- **Commit subjects follow [Conventional Commits](https://www.conventionalcommits.org/)** —
  `fix:`, `feat:`, `docs:`, `test:`, `build(deps):`, `ci:`. Nothing is generated from them; they
  are for the person reading the log.
- **`main` has linear history**, so pull requests are squashed or rebased, never merge-committed.
- **If a user would notice, add a changelog entry** in [`changelog/`](changelog/README.md), written
  for that user. Internals belong in the pull request and in `docs/ROADMAP.md`.

### Things that will fail CI if you forget them

- **Rust types that cross to the UI are generated, never hand-copied.** After changing one:

  ```bash
  cargo run -p xtask -- bindings
  ```

  `cargo test` fails while `ui/src/ipc/generated.ts` is stale, and names that command.
- **The CLI's output is a contract.** `crates/pipdock-cli/tests/golden.rs` holds a snapshot of every
  command. A diff there is a real behaviour change: re-bless it because you meant it, never to make
  the suite green.
- **Every piece of user-visible text lives in both languages** — `ui/src/locales/en` and
  `ui/src/locales/vi`. A string literal in JSX is a lint error, and a key missing from either
  language fails `i18n.test.ts`. Rust emits codes and data; it never phrases a sentence.
- **Never rewrite a file under `tests/fixtures/` with a text-mode tool.** They are byte-exact
  captures of engine output, line endings included.

## The rules that are not negotiable

These are what make PipDock safe to point at an environment someone depends on. A change that
weakens one will not be merged, however useful it is otherwise.

1. **Nothing mutates an environment without a plan the user accepted and a snapshot that was
   written.** If the snapshot fails, nothing runs.
2. **Subprocesses are started with argument arrays, never through a shell**, and package names and
   versions are validated before they reach one.
3. **PipDock does not resolve dependencies.** pip or uv resolves; PipDock explains.
4. **`--break-system-packages` is never passed unless the user turned the override on.**
5. **One failed package never aborts a batch**, and every failure carries exactly one catalog code.

[`docs/DATA-FLOW.md`](docs/DATA-FLOW.md) §9 and [`docs/SECURITY.md`](docs/SECURITY.md) are the full
statements.

## Accessibility

A change to the interface is not done until it works without a mouse. Before you open the pull
request:

- reach and operate everything new with the keyboard alone;
- check that focus is visible, and that it lands somewhere sensible when the screen changes or a
  dialog closes — never on nothing;
- give any control that shows only a symbol an accessible name, and every input a label;
- do not say anything by colour alone, and put `data-destructive` on a button that removes, forces
  or rewrites, so high contrast can still tell it from Cancel;
- keep `npm test` green: it runs [axe-core](https://github.com/dequelabs/axe-core) over every
  screen. axe checks structure; it cannot tell you what a screen reader says, so try one if you
  can — Narrator is `Ctrl+Win+Enter` — and say in the pull request if you could not.

[ACCESSIBILITY.md](ACCESSIBILITY.md) is the statement these keep true, and
[`docs/UI-SPEC.md`](docs/UI-SPEC.md) §8 is the keyboard map.

## Where things are

| Path | What is there |
|---|---|
| `crates/pipdock-core` | Everything that touches an environment: engines, plans, snapshots, the error catalog |
| `crates/pipdock-cli` | The `pipdock` command line, and its golden tests |
| `src-tauri` | The desktop shell and the IPC commands |
| `ui/src` | The React frontend; nothing in it calls Tauri's `invoke` directly, only the wrappers in `ui/src/ipc` |
| `docs/` | The design: read [`ARCHITECTURE.md`](docs/ARCHITECTURE.md) before a structural change |
| `legal/` | What a user agrees to. Edits here re-show the first-run screen once, by design |

## Licence

PipDock is [GPL-3.0-only](LICENSE). By contributing you agree that your contribution is licensed
under the same terms. Parts of PipDock were written with an AI assistant under human review;
[`docs/AI-DISCLOSURE.md`](docs/AI-DISCLOSURE.md) says how.
