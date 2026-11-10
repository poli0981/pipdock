# Accessibility

PipDock is meant to be usable by someone who uses a keyboard and no mouse, by someone who uses a
screen reader, and by someone who needs high contrast or larger text. This document says what that
means in practice, what has actually been tested and what has not, what is known not to work, and
how to tell us when something gets in your way.

**If you have hit a barrier, skip to [Reporting a barrier](#reporting-a-barrier).** You do not need
a GitHub account, and you do not need to diagnose the problem first.

## What we are working toward

The target is [WCAG 2.2](https://www.w3.org/TR/WCAG22/) Level AA, applied to a desktop application
and a command-line tool.

**This is a target, not a conformance claim.** PipDock has not been audited by anyone independent,
and the sections below list things that are known to fall short. What it has is a set of rules the
code is held to, tests that hold some of them, and one person's hands-on testing.

The priorities, in the order they are defended:

1. **Everything works from the keyboard.** No function needs a pointer.
2. **A screen reader is told what is on screen and what just changed.**
3. **Nothing is said by colour alone**, and text has enough contrast to read.
4. **The system's own settings are respected** — contrast themes, reduced motion, language.

## What is supported

### Keyboard

Every control is reachable and operable from the keyboard. The focus ring is a 2 px outline and is
never removed.

| Keys | What they do |
|---|---|
| `Tab` / `Shift+Tab` | Move between controls. In the package table each row is one stop |
| `Ctrl+1` … `Ctrl+9` | Go to a sidebar tab, by key position (works on layouts where digits need Shift) |
| `Ctrl+K` | Open the command palette; type to filter, `↑` `↓` to move, `Enter` to run, `Esc` to close |
| `/` | Go to Search and put the cursor in its field |
| `Space` | On a package row: select or deselect it |
| `Ctrl+A` | In the package table: select every row the screen is showing |
| `Enter` | On a package row: pin or unpin it. Never removes anything |
| `←` `→` | On a package row: move through its buttons (dependencies, pin, remove) |
| `←` `→` `Home` `End` | In Code Health: move between the tool tabs |
| `←` `→` `↑` `↓` | In a version conflict: move between *Keep compatible*, *Skip* and *Force latest*. Nothing is chosen until you press `Space` or `Enter` |
| `Esc` | Close a dialog, the palette or the console drawer; inside a package row, return to the row |

When the screen changes — a new tab, a plan's preview, its summary, the dependency view — focus
moves to the new screen's heading. When a dialog, the palette or the console drawer closes, focus
returns to what opened it. A button that is busy after you press it stays where focus is. Every
confirmation that removes, forces or rewrites something opens with focus on **Cancel**.

The full map and the reasoning behind it are in [`docs/UI-SPEC.md`](docs/UI-SPEC.md) §8.

### Screen readers

- Each screen has landmarks (header, navigation, main, footer) and one top-level heading.
- The package table is exposed as a grid with column headers, a row count and selection state.
- Buttons that show only a symbol have names. Status chips say what they mean in text.
- Dialogs are named by their heading and described by their body.
- A run announces its progress ("3 of 15 complete") once, not once per output line. Search
  announces that it is searching, how many results came back, and when nothing matched.
- Error messages are announced when they appear, and carry a code (`PD-XXX-000`) you can quote.
- The interface is in English and Vietnamese, and the document language follows the one in use.

### High contrast

Under a Windows contrast theme (forced colours), every colour in the interface is replaced by the
system's own. Because that removes every tint, the things a tint used to say are drawn another way:
destructive buttons get a double border, and the selected row, the selected search result and the
current sidebar tab get an outline.

PipDock follows the Windows display scale (Settings → Display → Scale), which enlarges the whole
window. It has no zoom of its own yet — see the limitations below.

### Colour, contrast and motion

- Text and status chips are checked against their backgrounds for at least 4.5:1 by a test that
  runs on every change. Status is always a word or a symbol as well as a colour.
- There is no animation in the interface. `prefers-reduced-motion` is honoured for anything that
  is ever added.

### The command line

`pipdock` prints plain text with no colour, no spinners and no lines that rewrite themselves. Every
command takes `--json` for one machine-readable document on standard output and `--quiet` to drop
progress; prompts, progress and warnings go to standard error; failures use documented exit codes.
Most of what the GUI does is available there — [`docs/CLI-GUIDE.md`](docs/CLI-GUIDE.md) lists the
few things that are not — so it is often a workable alternative while a GUI barrier is being fixed.

## What has been tested, and how

Honesty about this matters more than a long list.

| What | How | By whom, when |
|---|---|---|
| Every main flow with the mouse unplugged | By hand, on the installed 1.4.0 build, Windows 11 | The maintainer, November 2026. No blocking problem found |
| The same flows with Narrator | By hand, on the installed 1.4.0 build, Windows 11 | The maintainer, November 2026. No blocking problem found |
| Structure: roles, names, required parts | [axe-core](https://github.com/dequelabs/axe-core) over every screen, on every change | Automated, from 1.5.0 |
| Keyboard behaviour and where focus goes | Unit tests, on every change | Automated |
| Text contrast | Computed from the design tokens, on every change | Automated |
| Where focus goes, what the accessibility tree exposes, forced colours | In the installed build, driven by real key presses over the webview's debugging port, with forced colours emulated | During the 1.5.0 release. It found defects no test had; all are fixed |

**Not tested, and said so rather than implied:**

- **Narrator has not been run again on 1.5.0.** That release changed what a screen reader hears in
  several places (see the [changelog](changelog/1.x.md)). Those changes were checked against the
  accessibility tree, which says what is exposed — not against speech, which is what you hear.
- **NVDA and JAWS** have not been used with PipDock at all. Reports from people who use them are
  especially welcome.
- **Each Windows contrast theme by hand.** Forced colours were emulated; the four built-in themes
  were not each switched on.
- **Windows 10.** PipDock supports it, and the testing above was on Windows 11.
- **The installers.** The setup program and the MSI are produced by NSIS and WiX, and their
  accessibility is whatever those tools provide. It has not been assessed.
- Voice control, switch access and screen magnifiers.

## Known limitations

These are real, they are tracked in [`docs/ROADMAP.md`](docs/ROADMAP.md) under *Audit backlog*, and
where there is a workaround it is given.

- **One theme.** The interface is dark, with no light theme. A Windows contrast theme, light ones
  included, replaces every colour.
- **The package table is one tab stop per row.** There is no `↑` / `↓` between rows yet, so
  passing a long table by `Tab` is slow. *Workaround:* `Ctrl+1` … `Ctrl+9` jump past it, and the
  Updates tab lists only what is outdated.
- **Only the rows on screen exist.** The table draws about 25 rows at a time, so a screen reader's
  own "list all rows" finds only those. The row count and each row's position are exposed.
- **Live output is not read line by line.** The console drawer shows an engine's output as it
  arrives and deliberately does not announce it; progress is announced instead. The text is there
  to read at your own pace.
- **Some announcements are unverified.** A few status messages appear together with the part of
  the screen that carries them, and several errors can appear at once after a failed batch.
  Whether a screen reader says them, and in what order, has not been confirmed on 1.5.0.
- **A few failures say nothing.** If saving a report, copying to the clipboard or opening a file
  picker fails, the screen does not change. Nothing is lost, but nothing tells you either.
- **Why a control is disabled is sometimes only in a tooltip**, which a keyboard cannot reach
  because a disabled control cannot be focused.
- **No zoom inside the application.** It was built for 1.5.0 and held back: at 200 % in the
  default window the package-name column collapsed to nothing and the last sidebar tabs were cut
  off, which is worse than no zoom. *Workaround:* the Windows display scale enlarges everything
  correctly.
- **Long paths and package names are cut off** rather than wrapped in a few places, most of all
  in a narrow window.
- **Dates and sizes are not formatted for the interface language.**
- **`Ctrl+K` is matched by the letter the layout types**, unlike the digit shortcuts, so it moves
  on a layout where K is elsewhere.
- **The legal documents open in your browser**, on GitHub, rather than inside the application.

## Reporting a barrier

Either of these reaches the same person:

- **[Open an accessibility issue](https://github.com/poli0981/pipdock/issues/new?template=accessibility.yml)**
  on GitHub, or
- write to **[accessibility@poli0981.dev](mailto:accessibility@poli0981.dev)** — no account needed.

Write in English or Vietnamese — tiếng Việt cũng được. It helps to include:

- what you were trying to do, and what happened instead;
- the PipDock version (**About → This build**) and your Windows version;
- the assistive technology you use and its version, if any — Narrator, NVDA, a contrast theme,
  zoom level, keyboard only;
- whether it stops you completely or slows you down.

You do not have to work out the cause, name a WCAG criterion or propose a fix. "I could not get
past the first screen" is a complete report.

### What happens next

- **You get an answer within 7 days**, the same commitment as for security reports.
- A barrier that stops someone using PipDock is treated as a bug of the highest priority and goes
  into the next release. Others are scheduled like any other defect and recorded in the roadmap.
- If there is a workaround — often the command line — you will be told what it is straight away.
- You will be told when a fix ships, and asked whether it worked for you.

**There is no fixed deadline for a fix.** PipDock is made by one person, and a promised date that
slips is worse than none. What is promised is the answer, the priority, and that a report is never
closed as "works for me" without being tried the way you described it.

## For contributors

A change to the interface is not finished until it works without a mouse.
[`CONTRIBUTING.md`](CONTRIBUTING.md) has the checklist, and the pull request template repeats it.
In short: reach it and operate it by keyboard, keep the focus ring, put focus somewhere sensible
when the screen changes, name anything that shows only a symbol, never say something by colour
alone, mark a destructive button `data-destructive`, add copy in both languages, and keep
`npm test` green — it runs axe over every screen.

## Ownership

Accessibility reports go to the maintainer, [@poli0981](https://github.com/poli0981), who triages
them, keeps the list of known limitations above true, and updates this document with each release
that changes it.

Last reviewed: 2026-11-10, for PipDock 1.5.0.

---

The structure of this document follows the
[Accessibility Toolkit's `ACCESSIBILITY.md`](https://github.com/open-source-accessibility/accessibility-toolkit/blob/main/ACCESSIBILITY.md)
(MIT), following GitHub's
[guide to accessibility in open source](https://opensource.guide/accessibility-best-practices-for-your-project/).
