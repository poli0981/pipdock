<!--
One pull request is one slice: a single thing, taken deep, with its tests. CONTRIBUTING.md has the
details. Delete the sections that do not apply.
-->

## What this changes

<!-- What a user or a script would notice, and why. Link the issue or the docs/ROADMAP.md entry. -->

## How it was checked

<!-- The commands you ran, and what you did in the running app. "Tests pass" is not the whole answer:
     most defects in this project were found by running it. -->

- [ ] A test fails without this change and passes with it
- [ ] `npm run lint`, `npm run typecheck`, `npm test`
- [ ] `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --locked`

## If it touches…

- [ ] **A Rust type that crosses to the UI** — bindings regenerated (`cargo run -p xtask -- bindings`)
- [ ] **CLI output** — goldens re-blessed deliberately, and the diff is the change you meant
- [ ] **User-visible text** — added to both `ui/src/locales/en` and `ui/src/locales/vi`; nothing phrased in Rust
- [ ] **A new error code** — every place `CLAUDE.md` lists, both `errors.json` included
- [ ] **Behaviour a user reads about** — `docs/` updated, and `legal/` checked separately
- [ ] **Something a user would notice** — an entry in `changelog/`

## Accessibility (for any change to the interface)

- [ ] Everything new can be reached and operated with the keyboard alone
- [ ] Focus is visible, and goes somewhere sensible when the screen changes or a dialog closes
- [ ] Controls that show only a symbol have an accessible name; inputs have labels
- [ ] Nothing is said by colour alone, and a destructive button carries `data-destructive`
- [ ] Checked once with a screen reader, or said here that it was not

<!-- ACCESSIBILITY.md is the statement these keep true. -->
