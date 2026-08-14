# PipDock — Privacy Policy

*Version 1.1 · Effective 2026-08-14 · Governing language: English.*

## 1. The short version

**PipDock collects nothing.** No telemetry, no analytics, no crash reporting, no accounts, no identifiers. The developer receives no data from your use of the application unless you personally choose to open a bug report.

## 2. Data stored locally on your device

All application data lives under `%LOCALAPPDATA%\PipDock\data\` and never leaves your machine:

| Data | Purpose |
|---|---|
| Settings (engine, locale, thresholds, consent record) | app configuration |
| Environment list & package index cache | discovery and instant search |
| Snapshots (package lists of your environments) | rollback |
| Pins | update exclusions |
| Log files (14-day rotation) | troubleshooting; may contain package names and file paths |

Deleting the `data\` folder is a complete reset. **Delete `data\`, not its parent:** PipDock's own program files are installed in `%LOCALAPPDATA%\PipDock\`, so removing that folder uninstalls the application as well — use the uninstaller for that. No data is synced or backed up anywhere by PipDock.

## 3. Network connections

PipDock connects to exactly one destination: **pypi.org / files.pythonhosted.org**, for the package name index and per-package metadata — standard web requests carrying no personal data beyond your IP address, as with any website.

**PipDock does not update itself**, so it never contacts GitHub or anywhere else to look for a new version. New versions are downloaded by you from the Releases page.

Some buttons hand a URL to your browser rather than fetching anything: the legal documents and the bug-report form on **github.com**, ruff's rule pages on **docs.astral.sh**, and the author's contact page on **poli0981.dev**. Those are visits your browser makes, under its own settings, only after you click.

The pip/uv engines make their own connections to the package indexes configured on your system (including any private indexes you have set up); PipDock does not alter or observe those configurations beyond invoking the engines.

## 4. Bug reports (opt-in, manual)

The in-app **Report bug** button prepares a GitHub issue in your browser, prefilled with app version, Windows version, engine and Python versions, the error code, and a short log excerpt; the full log is copied to your clipboard. **Nothing is transmitted until you personally review and submit the issue on GitHub**, under GitHub's own terms and privacy policy. Check the excerpt for paths or names you consider private before submitting — the template reminds you to.

## 5. Children

PipDock is a developer tool and is not directed at children; it processes no personal data regardless of the user's age.

## 6. Changes

Material changes to this policy bump the legal documents' version hash, and the application will present the legal gate again before continuing. History is visible in the repository's git log.

## 7. Contact

Privacy questions: email <contact@poli0981.dev>, or open a discussion or issue at <https://github.com/poli0981/pipdock>. The same contact details are listed in the application under **About**.

---

## Tóm tắt Tiếng Việt (tham khảo)

PipDock **không thu thập bất kỳ dữ liệu nào**: không telemetry, không analytics, không tài khoản. Mọi dữ liệu (cài đặt, cache chỉ mục, snapshot, log) nằm trong `%LOCALAPPDATA%\PipDock\data\` trên máy bạn; xóa thư mục `data\` là xóa sạch — **đừng xóa thư mục cha**, vì chính chương trình được cài ở đó và xóa nó là gỡ luôn ứng dụng. Ứng dụng **chỉ** kết nối tới pypi.org (dữ liệu gói); PipDock không tự cập nhật nên không bao giờ gọi về GitHub. Một vài nút giao URL cho trình duyệt của bạn mở (github.com, docs.astral.sh, poli0981.dev) — đó là trình duyệt truy cập, sau khi bạn bấm. Nút báo lỗi chỉ mở form GitHub đã điền sẵn — không gì được gửi đi cho tới khi bạn tự xem lại và bấm gửi. Liên hệ: <contact@poli0981.dev>.
