# PipDock — Disclaimer

*Version 1.0 · Effective 2026-07-17 · Governing language: English.*

## 1. Modifying environments carries inherent risk

PipDock installs, upgrades, and removes Python packages. Even with previews, dependency resolution, snapshots, and post-checks, **any change to an environment can break projects that depend on it** — including in ways no tool can predict (runtime behavior changes between package versions, compiled-extension ABI issues, packages with incorrect metadata). Use the preview step, keep snapshots, and test your projects after significant updates. Do not run bulk updates on environments backing production systems without your own backups and change process.

## 2. Forced operations are at your own risk

The "Force latest" and forced-uninstall options exist for expert use. When you choose them, PipDock shows which requirements will be violated; proceeding **knowingly puts the environment into a state its own metadata declares broken**. The resulting behavior of affected packages is undefined.

## 3. System Python

PipDock blocks modification of externally-managed (PEP 668) Python installations by default. If you enable the override, you accept that modifying an OS-managed or vendor-managed Python can damage tooling on your system beyond Python projects.

## 4. Information accuracy

Package metadata, latest-version information, vulnerability findings (where enabled), and code-health reports are derived from third-party sources (PyPI, the OSV database, deptry/vulture/ruff output) and from heuristics such as reverse-dependency analysis. They may be incomplete, outdated, or wrong — for example, dead-code detection produces false positives on dynamically referenced code. **Review before acting**; PipDock's reports are assistance, not authority.

## 5. Not professional advice; no affiliation

Nothing in PipDock constitutes professional, security, or legal advice. PipDock is an independent project, not affiliated with or endorsed by the Python Software Foundation (Python, pip, PyPI), Astral (uv, ruff), or the authors of deptry, vulture, pip-audit, or any package it manages.

## 6. Warranty and liability

The no-warranty and limitation-of-liability terms of the GPL-3.0 (§15–16), restated in the [EULA](EULA.md), apply to all of the above.

---

## Tóm tắt Tiếng Việt (tham khảo)

Mọi thay đổi lên môi trường Python đều có rủi ro làm hỏng dự án phụ thuộc vào nó, kể cả khi đã xem trước và có snapshot — hãy tự sao lưu và kiểm thử sau khi cập nhật lớn. Tùy chọn "Ép bản mới nhất" và gỡ cưỡng bức đặt môi trường vào trạng thái mà chính metadata của nó tuyên bố là hỏng — bạn tự chịu rủi ro. Thông tin phiên bản, lỗ hổng, dead code đến từ nguồn bên thứ ba và có thể sai — hãy xem xét trước khi hành động. PipDock độc lập, không liên kết với PSF hay Astral. Điều khoản miễn trừ bảo hành theo GPL-3.0 §15–16 áp dụng.
