# PipDock — End User License Agreement (Terms of Use)

*Version 1.1 · Effective 2026-08-14 · Governing language: English (bản Tiếng Việt tóm tắt ở cuối chỉ mang tính tham khảo).*

## 1. License

PipDock is free software, licensed under the **GNU General Public License, version 3** (GPL-3.0). The full license text is available in the repository ([LICENSE](../LICENSE)) and at <https://www.gnu.org/licenses/gpl-3.0.html>. Your rights to use, study, modify, and redistribute PipDock come from the GPL-3.0, and **nothing in this document adds restrictions to, or removes rights granted by, that license** (per GPL-3.0 §7, any such added restriction would be void). This document only (a) restates the GPL's warranty and liability terms as they apply to you as a user, and (b) records acknowledgments the application asks for at first run.

## 2. What PipDock does (acknowledgment)

PipDock installs, upgrades, and removes packages in Python environments **at your direction**, by invoking third-party package managers (pip, uv) and analysis tools (deptry, vulture, ruff; pip-audit is planned and is not invoked in version 1.0). These operations modify software environments on your computer and can affect the behavior of your projects. You are responsible for reviewing each preview and confirmation the application presents before applying changes.

## 3. No warranty

As stated in GPL-3.0 §15: THERE IS NO WARRANTY FOR THE PROGRAM, TO THE EXTENT PERMITTED BY APPLICABLE LAW. THE PROGRAM IS PROVIDED "AS IS" WITHOUT WARRANTY OF ANY KIND, EITHER EXPRESSED OR IMPLIED, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE. THE ENTIRE RISK AS TO THE QUALITY AND PERFORMANCE OF THE PROGRAM IS WITH YOU.

## 4. Limitation of liability

As stated in GPL-3.0 §16: IN NO EVENT, UNLESS REQUIRED BY APPLICABLE LAW OR AGREED TO IN WRITING, WILL ANY COPYRIGHT HOLDER BE LIABLE TO YOU FOR DAMAGES, INCLUDING ANY GENERAL, SPECIAL, INCIDENTAL OR CONSEQUENTIAL DAMAGES ARISING OUT OF THE USE OR INABILITY TO USE THE PROGRAM (INCLUDING BUT NOT LIMITED TO LOSS OF DATA OR DATA BEING RENDERED INACCURATE OR LOSSES SUSTAINED BY YOU OR THIRD PARTIES OR A FAILURE OF THE PROGRAM TO OPERATE WITH ANY OTHER PROGRAMS).

## 5. Third-party software and services

Packages you choose to install come from the Python Package Index or other indexes you have configured; they are published by third parties under their own licenses, and installing them may execute their build code. PipDock does not review, endorse, or warrant any third-party package. Third-party components bundled with or invoked by PipDock are listed in [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

## 6. Trademarks

"Python", "pip", and "PyPI" are trademarks of the Python Software Foundation. "uv" and "ruff" are projects of Astral. PipDock is an independent project and is not affiliated with, sponsored, or endorsed by these organizations.

## 7. Acceptance record

The application records your acceptance of this document (together with the Disclaimer and Privacy Policy) locally on your device, keyed to the documents' version hash. If these documents change materially, the application will ask again. You can read them at any time from the **About** tab, which also shows the hash your acceptance was recorded against.

---

## Tóm tắt Tiếng Việt (tham khảo)

PipDock là phần mềm tự do theo giấy phép GPL-3.0; tài liệu này không thêm bất kỳ hạn chế nào ngoài GPL. Ứng dụng thay đổi các môi trường Python **theo lệnh của bạn** thông qua pip/uv; phần mềm được cung cấp "nguyên trạng", không bảo hành, và các tác giả không chịu trách nhiệm cho thiệt hại phát sinh (theo GPL-3.0 §15–16). Gói bên thứ ba do bạn chọn cài thuộc trách nhiệm của nhà phát hành gói đó. Việc chấp thuận được lưu cục bộ trên máy bạn.
