#!/usr/bin/env python3
"""生成 GitHub Release 正文：产物下载表（含大小与 SHA256）+ 本次 tag 的提交清单。

表是扫 --artifacts-dir 得到的，所以正文里的链接必然与实际产物一一对应
（不需要像手写清单那样再加一道"链接校验"）。

用法（CI 里由 release job 调用，需要完整 git 历史才能取到上一个 tag）：
    python3 scripts/release/gen_release_notes.py --tag v0.1.0 --version 0.1.0 \\
        --repo SpringHgui/onecrt --artifacts-dir artifacts --output release_body.md
"""

import argparse
import hashlib
import os
import subprocess
import sys

# 只列面向用户的安装包；.sig 与 latest.json 是给程序读的，不进下载表
SKIP_SUFFIX = (".sig", "latest.json")

KIND = [
    ("-windows-x64-setup.exe", "Windows x64 安装包（NSIS）"),
    ("-linux-x64.deb", "Linux x64 安装包（deb）"),
    ("-linux-x64.AppImage", "Linux x64 免安装（AppImage，支持应用内更新）"),
    ("-macos-arm64.dmg", "macOS Apple Silicon 磁盘镜像"),
    ("-macos-arm64.app.tar.gz", "macOS Apple Silicon 应用包（应用内更新用）"),
    ("-macos-x64.dmg", "macOS Intel 磁盘镜像"),
    ("-macos-x64.app.tar.gz", "macOS Intel 应用包（应用内更新用）"),
]


def kind_of(name: str) -> str:
    for suffix, label in KIND:
        if name.endswith(suffix):
            return label
    return name


def sha256(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def git(*args: str) -> str:
    # encoding 显式 utf-8：提交信息是中文，Windows 上默认按 GBK 解会直接抛 UnicodeDecodeError
    r = subprocess.run(["git", *args], capture_output=True, text=True, encoding="utf-8", errors="replace")
    if r.returncode != 0:
        print(f"git {' '.join(args)} 失败：{(r.stderr or '').strip()}", file=sys.stderr)
        raise SystemExit(1)
    return r.stdout.strip()


def commits(tag: str) -> str:
    """上一个 tag 到本次 tag 之间的提交；首个版本没有前 tag，就列全部。"""
    prev = subprocess.run(
        ["git", "describe", "--tags", "--abbrev=0", f"{tag}^"],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    range_ = f"{prev.stdout.strip()}..{tag}" if prev.returncode == 0 else tag
    log = git("log", "--no-merges", "--first-parent", "--pretty=format:- %s (%h)", range_)
    return log or "- 首个版本"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--tag", required=True)
    ap.add_argument("--version", required=True)
    ap.add_argument("--repo", required=True)
    ap.add_argument("--artifacts-dir", required=True)
    ap.add_argument("--output", required=True)
    a = ap.parse_args()

    names = sorted(
        n for n in os.listdir(a.artifacts_dir) if not n.endswith(SKIP_SUFFIX) and not n.startswith(".")
    )
    if not names:
        print("artifacts 目录是空的，没什么可发的", file=sys.stderr)
        return 1

    base = f"https://github.com/{a.repo}/releases/download/{a.tag}"
    lines = [f"## 下载 v{a.version}", ""]
    lines.append("| 文件 | 说明 | 大小 | SHA256 |")
    lines.append("| --- | --- | --- | --- |")
    for n in names:
        p = os.path.join(a.artifacts_dir, n)
        size = f"{os.path.getsize(p) / (1024 * 1024):.1f} MB"
        lines.append(f"| [{n}]({base}/{n}) | {kind_of(n)} | {size} | `{sha256(p)[:16]}…` |")

    lines += [
        "",
        "## 安装说明",
        "",
        "- **Windows**：装 `*-windows-x64-setup.exe`。未做代码签名，首启会有 SmartScreen 提示，点「仍要运行」即可。",
        "- **Linux**：Debian/Ubuntu 用 `*.deb`（`sudo apt install ./包名.deb`）；其他发行版用 `*.AppImage`（`chmod +x` 后直接运行）。",
        "- **macOS**：装 `*.dmg`。未做 Apple 公证，首次打开需在「系统设置 → 隐私与安全性」里允许，或 `xattr -dr com.apple.quarantine /Applications/shellx.app`。",
        "",
        "应用内更新（「帮助 → 关于 → 检查更新」）走本 Release 的 `latest.json`：Windows 与 Linux(AppImage) 可直接自助升级；"
        "macOS 的自动更新需要 Developer ID 签名，未签名的包请手动下载 dmg。",
        "",
        "## 本次变更",
        "",
        commits(a.tag),
        "",
    ]

    with open(a.output, "w", encoding="utf-8") as f:
        f.write("\n".join(lines))
    print(f"已写入 {a.output}（{len(names)} 个产物）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
