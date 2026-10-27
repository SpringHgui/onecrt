#!/usr/bin/env python3
"""把构建产物拼成 Tauri updater 的更新清单 latest.json。

输入是 release job 汇总下来的产物目录：每个可更新资产旁边都有同名的 `.sig`
（minisign 分离签名，构建期由 `--config '{"bundle":{"createUpdaterArtifacts":true}}'` 出）。
资产按 `<产品>-<版本>-<平台槽>.<扩展>` 约定命名，平台槽决定 latest.json 里的 key，
所以改名/漏传在这里就会表现为"某个平台没有更新源"，脚本会为此报错而不是静默出半份清单。

用法（CI 里由 release job 调用）：
    python3 scripts/release/gen_updater_manifest.py --version 0.1.0 --tag v0.1.0 \\
        --repo SpringHgui/onecrt --artifacts-dir artifacts \\
        --require windows-x86_64,linux-x86_64,darwin-aarch64 --output artifacts/latest.json
"""

import argparse
import json
import os
import sys
from datetime import datetime, timezone

# 资产文件名后缀 → Tauri 的平台 key。后缀而不是全名匹配：版本号在中间会变。
SUFFIX_PLATFORM = [
    ("-windows-x64-setup.exe", "windows-x86_64"),
    ("-linux-x64.AppImage", "linux-x86_64"),
    ("-macos-arm64.app.tar.gz", "darwin-aarch64"),
    ("-macos-x64.app.tar.gz", "darwin-x86_64"),
]


def platform_of(asset: str):
    for suffix, key in SUFFIX_PLATFORM:
        if asset.endswith(suffix):
            return key
    return None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", required=True)
    ap.add_argument("--tag", required=True)
    ap.add_argument("--repo", required=True, help="owner/name，用于拼下载地址")
    ap.add_argument("--artifacts-dir", required=True)
    ap.add_argument("--notes", default="", help="客户端可展示的更新说明，默认用版本号")
    ap.add_argument(
        "--require",
        default="",
        help="逗号分隔的平台 key，缺任意一个就失败（本仓构建哪几个平台由工作流说了算）",
    )
    ap.add_argument("--output", required=True)
    a = ap.parse_args()

    platforms = {}
    for name in sorted(os.listdir(a.artifacts_dir)):
        if not name.endswith(".sig"):
            continue
        asset = name[: -len(".sig")]
        key = platform_of(asset)
        if key is None:
            print(f"跳过不参与在线更新的签名：{name}", file=sys.stderr)
            continue
        with open(os.path.join(a.artifacts_dir, name), encoding="utf-8") as f:
            signature = f.read().strip()
        if not signature:
            print(f"{name} 是空的——私钥没配好或签名被截断", file=sys.stderr)
            return 1
        platforms[key] = {
            "url": f"https://github.com/{a.repo}/releases/download/{a.tag}/{asset}",
            "signature": signature,
        }

    if not platforms:
        print("artifacts 里没有任何可更新资产的 .sig，不出清单", file=sys.stderr)
        return 1
    wanted = [k.strip() for k in a.require.split(",") if k.strip()]
    missing = [k for k in wanted if k not in platforms]
    if missing:
        print(f"更新清单缺平台：{', '.join(missing)}（对应 job 失败或产物没下载下来）", file=sys.stderr)
        return 1

    manifest = {
        "version": a.version,
        "notes": a.notes or f"OneCRT {a.version}",
        "pub_date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": platforms,
    }
    with open(a.output, "w", encoding="utf-8") as f:
        json.dump(manifest, f, ensure_ascii=False, indent=2)
        f.write("\n")
    print("已写入 " + a.output + "，平台：" + ", ".join(sorted(platforms)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
