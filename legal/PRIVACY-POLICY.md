# PipDock — Privacy Policy

*Version 1.0 · Effective 2026-07-17 · Governing language: English.*

## 1. The short version

**PipDock collects nothing.** No telemetry, no analytics, no crash reporting, no accounts, no identifiers. The developer receives no data from your use of the application unless you personally choose to open a bug report.

## 2. Data stored locally on your device

All application data lives under `%LOCALAPPDATA%\PipDock\` and never leaves your machine:

| Data | Purpose |
|---|---|
| Settings (engine, locale, thresholds, consent record) | app configuration |
| Environment list & package index cache | discovery and instant search |
| Snapshots (package lists of your environments) | rollback |
| Pins | update exclusions |
| Log files (14-day rotation) | troubleshooting; may contain package names and file paths |

Deleting this folder is a complete reset. No data is synced or backed up anywhere by PipDock.

## 3. Network connections

PipDock connects only to: **pypi.org / files.pythonhosted.org** (package name index and metadata — standard web requests carrying no personal data beyond your IP address, as with any website) and **github.com** (checking for and downloading PipDock's own updates). The pip/uv engines make their own connections to the package indexes configured on your system (including any private indexes you have set up); PipDock does not alter or observe those configurations beyond invoking the engines.

## 4. Bug reports (opt-in, manual)

The in-app **Report bug** button prepares a GitHub issue in your browser, prefilled with app version, Windows version, engine and Python versions, the error code, and a short log excerpt; the full log is copied to your clipboard. **Nothing is transmitted until you personally review and submit the issue on GitHub**, under GitHub's own terms and privacy policy. Check the excerpt for paths or names you consider private before submitting — the template reminds you to.

## 5. Children

PipDock is a developer tool and is not directed at children; it processes no personal data regardless of the user's age.

## 6. Changes

Material changes to this policy bump the legal documents' version hash, and the application will present the legal gate again before continuing. History is visible in the repository's git log.

## 7. Contact

Privacy questions: open a discussion or issue at <https://github.com/poli0981/pipdock>.

---

## Tóm tắt Tiếng Việt (tham khảo)

PipDock **không thu thập bất kỳ dữ liệu nào**: không telemetry, không analytics, không tài khoản. Mọi dữ liệu (cài đặt, cache chỉ mục, snapshot, log) nằm trong `%LOCALAPPDATA%\PipDock\` trên máy bạn; xóa thư mục này là xóa sạch. Ứng dụng chỉ kết nối tới pypi.org (dữ liệu gói) và github.com (cập nhật chính PipDock). Nút báo lỗi chỉ mở form GitHub đã điền sẵn — không gì được gửi đi cho tới khi bạn tự xem lại và bấm gửi.
