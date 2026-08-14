# Security Policy

## Supported versions

The latest release gets fixes. The previous minor gets security-only patches for 90 days. There is
no update channel — PipDock does not update itself, so a fix reaches you when you download the new
build from [Releases](https://github.com/poli0981/pipdock/releases).

## Reporting a vulnerability

**Do not open a public issue.** Use either:

- [GitHub Security Advisories](https://github.com/poli0981/pipdock/security/advisories/new) — preferred
- `code@poli0981.dev`

Target acknowledgment is 7 days. Coordinated disclosure is preferred; tell us if you have a
disclosure deadline and we will work to it.

Useful in a report: the PipDock version (it is on the **About** tab), the Windows version, the
engine (pip or uv) and its version, and the smallest sequence of steps that reproduces the issue.

## What is in scope

PipDock runs subprocesses against Python interpreters you point it at, so the interesting surface is
where untrusted input reaches a command line, a file path, or a URL handed to your browser.
[`docs/SECURITY.md`](../docs/SECURITY.md) is the full threat model — §2 (argv arrays, never a shell),
§3 (PEP 668), §4 (network and the opener allowlist) and §7 (supply chain) are the sections most
reports land in.

Out of scope: vulnerabilities in the packages PipDock installs on your behalf — report those to the
package's own maintainers — and in pip, uv, deptry, vulture or ruff, which PipDock invokes but does
not distribute.
