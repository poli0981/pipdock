# PipDock

[![CI · Rust](https://github.com/poli0981/pipdock/actions/workflows/ci-rust.yml/badge.svg)](https://github.com/poli0981/pipdock/actions/workflows/ci-rust.yml)
[![CI · Node](https://github.com/poli0981/pipdock/actions/workflows/ci-node.yml/badge.svg)](https://github.com/poli0981/pipdock/actions/workflows/ci-node.yml)
[![CodeQL](https://github.com/poli0981/pipdock/actions/workflows/codeql.yml/badge.svg)](https://github.com/poli0981/pipdock/actions/workflows/codeql.yml)
[![Release](https://img.shields.io/github/v/release/poli0981/pipdock?display_name=tag&sort=semver)](https://github.com/poli0981/pipdock/releases)
[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-blue)](LICENSE)

**A friendly dock for your Python environments.** Inspect, install, update, and clean up Python packages in bulk — through a keyboard-first GUI or a scriptable CLI — without ever hand-typing another `pip install` incantation or guessing why an upgrade broke your project.

> Repo: `poli0981/pipdock` · License: **GPL-3.0-only** · Platform: **Windows 10/11** · Contact: [contact@poli0981.dev](mailto:contact@poli0981.dev) · [other channels](https://poli0981.dev/links/)

---

## Why PipDock

`pip` is powerful but unforgiving: it does not preview what an upgrade will change, `pip uninstall` removes packages that other packages still need without a word of warning, and a failed bulk update leaves you guessing what state your environment is in. PipDock wraps the official resolvers (pip or uv — your choice) with:

- **Preview before touch** — every install/update runs a dry-run resolve first and shows you exactly what will change.
- **Three-way conflict handling** — when the latest version of a package would break another, choose *compatible version* (safe default), *skip*, or *force latest* with a clear "this will break X" warning.
- **Snapshots & rollback** — a `pip freeze` snapshot is taken before every batch operation; one click restores it.
- **Reverse-dependency guard** — uninstalling a package that others depend on triggers a warning listing exactly what would break.
- **Skip-and-continue execution** — one failed package never aborts the batch; you get a final report: *"13 successful, 2 failed"* with per-package reasons.
- **Never touches itself** — PipDock is a standalone binary that lives outside every environment it manages, so it can never break itself mid-update.

## Features (v1)

| Area | What you get |
|---|---|
| Environments | Auto-discovery of venvs / uv venvs / system Pythons (PEP 514 + `py` launcher), manual add, PEP 668 guard |
| Installed | Full package list per env; up-to-date packages dimmed, outdated badged |
| Updates | Bulk update with group dry-run resolve, held-back explanations, 3-way conflict choices |
| Search & Install | Instant fuzzy search over a locally cached PyPI name index (PEP 691), metadata on demand, install queue |
| Uninstall | Bulk uninstall with reverse-dependency warnings |
| Pins | Exclude packages from bulk updates; auto-suggest pinning heavily-depended-on packages |
| Snapshots | Automatic pre-batch snapshots, diff view, one-click rollback |
| Engine | pip **or** uv, selectable in Settings (uv auto-detected on first run) |
| Code Health | deptry + vulture + ruff against a project folder — unused deps, dead code, style — run from PipDock's own isolated tools env |
| pip upkeep | Check & update pip itself per environment |
| CLI | `pipdock` — full parity for core operations, JSON output, CI-friendly exit codes |
| i18n | English + Tiếng Việt |
| Privacy | No telemetry. **PyPI is the only host PipDock connects to.** It does not check for updates, so it never calls home — GitHub and the ruff docs are only ever handed to your browser as a link you clicked |

## Install

Download the installer (NSIS `.exe` or `.msi`) from [GitHub Releases](https://github.com/poli0981/pipdock/releases).

PipDock does not update itself — grab a new build from the same page when you want one. That is deliberate: a self-updater is a standing remote-code-execution path into a tool that already runs subprocesses against your interpreters, and it is not worth the convenience (see [docs/SECURITY.md](docs/SECURITY.md) §5).

> Windows SmartScreen may warn on first run because the binaries are not EV-code-signed. Every release publishes `SHA256SUMS.txt`; verify your download against it before running:
>
> ```powershell
> (Get-FileHash .\PipDock_1.0.0_x64-setup.exe -Algorithm SHA256).Hash.ToLower()
> ```
>
> Compare that against the matching line in `SHA256SUMS.txt`. If they differ, do not run the file.

## Quick start

**GUI:** launch PipDock → accept the first-run legal gate → pick an environment (auto-discovered or *Browse…*) → the *Updates* tab shows what's outdated → *Select all* → *Update* → review the preview → *Confirm*. Four clicks.

**CLI:** the installer puts `pipdock.exe` next to the GUI, in `%LOCALAPPDATA%\PipDock\`.
**PipDock does not edit your `PATH`**; add that folder yourself if you want to type `pipdock`
from anywhere:

```powershell
$dir = "$env:LOCALAPPDATA\PipDock"
[Environment]::SetEnvironmentVariable('Path', "$([Environment]::GetEnvironmentVariable('Path','User'));$dir", 'User')
```

```text
pipdock env list
pipdock update --all --env C:\proj\.venv --strategy compatible --yes
pipdock doctor
pipdock snapshot rollback latest
```

## Documentation

| Doc | Purpose |
|---|---|
| [docs/PRD.md](docs/PRD.md) | Product goals, personas, feature matrix P0–P2, non-goals |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Crate layout, Engine trait, IPC surface, storage |
| [docs/DATA-FLOW.md](docs/DATA-FLOW.md) | State machines for update / install / uninstall, engine command mapping |
| [docs/UI-SPEC.md](docs/UI-SPEC.md) | Terminal-tech design system, screens, click budgets |
| [docs/CLI-SPEC.md](docs/CLI-SPEC.md) | Commands, flags, exit codes, JSON contracts |
| [docs/CODE-HEALTH-SPEC.md](docs/CODE-HEALTH-SPEC.md) | deptry / vulture / ruff integration |
| [docs/ERROR-CATALOG.md](docs/ERROR-CATALOG.md) | Error codes, stderr detection patterns, user guidance |
| [docs/SECURITY.md](docs/SECURITY.md) | Threat model, PEP 668 policy, supply-chain hygiene |
| [docs/TESTING.md](docs/TESTING.md) | Test strategy incl. disposable-venv integration tests |
| [docs/RELEASE-CI.md](docs/RELEASE-CI.md) | CI callers into `poli0981/.github`, release pipeline |
| [docs/I18N.md](docs/I18N.md) | EN/VI localization rules |
| [docs/ROADMAP.md](docs/ROADMAP.md) | Spike week, milestones, exit criteria |
| [legal/](legal/) | EULA, Disclaimer, Privacy Policy, Third-Party Notices (linked by the in-app legal gate) |

## License

Copyright © 2026 Kokone (poli0981). PipDock is free software under **GPL-3.0-only** — see [LICENSE](LICENSE). The legal documents in [legal/](legal/) are committed publicly so the in-app legal gate and the About tab can link to them on GitHub.

PipDock is an independent project. It is not affiliated with or endorsed by the Python Software Foundation (Python, pip, PyPI) or Astral (uv, ruff).

---

## Tiếng Việt (tóm tắt)

PipDock là ứng dụng Windows (GUI + CLI) giúp kiểm tra, cài đặt, cập nhật và gỡ **hàng loạt** gói thư viện Python một cách an toàn: luôn chạy dry-run xem trước thay đổi, xử lý xung đột phiên bản với 3 lựa chọn rõ ràng, tự chụp snapshot trước mỗi lần chạy để rollback một chạm, cảnh báo khi gỡ gói mà gói khác đang phụ thuộc, và tổng kết "13 thành công, 2 thất bại" kèm lý do từng gói. Hỗ trợ engine pip hoặc uv (chọn trong Settings), tìm gói tức thì nhờ cache chỉ mục PyPI, kèm module Code Health (deptry/vulture/ruff) để dọn dependency thừa và dead code. Không thu thập dữ liệu. Giấy phép GPL-3.0.
