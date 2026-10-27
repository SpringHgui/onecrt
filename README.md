<div align="center">
  <img src="public/app-icon.png" width="128" height="128" alt="OneCRT">

  # OneCRT
  ### 跨平台 SSH2 终端客户端

  <img src="site/assets/shot-terminal-dark.png" width="880" alt="深色主题主界面：左侧会话管理器按生产 / 预发分组，web-01 已选中；中间 web-01 终端标签页里跑 kubectl、df、systemctl；状态栏显示 root@203.0.113.11:22 已连接"><br>
  <sub>会话树 + 终端标签页</sub><br><br>
  <img src="site/assets/shot-files-dark.png" width="880" alt="文件面板（SFTP）：左栏本地 C:\Users\demo\deploy 列出 certs、releases、scripts、compose.yaml、nginx.conf、app-2026.10.26.tgz、deploy.sh；右栏远端 root@203.0.113.11 的 /srv/shop 列出 app、config、public、storage、artisan、deploy.log、VERSION；下方是空的传输区"><br>
  <sub>文件面板（SFTP）与传输区</sub>
</div>

安装包在 [Releases](https://github.com/SpringHgui/onecrt/releases)，当前 v0.2.0：Windows（NSIS）、Linux（deb + AppImage）、macOS（Apple Silicon dmg）。

## 功能

- **会话管理**：文件夹 + 会话树，支持过滤、双击连接、右键菜单（剪切 / 复制 / 粘贴、查找、重命名、删除、属性），树结构存 JSON。
- **导入外部配置**：读 SecureCRT 或 WinSCP 导出的会话文件，连同分组层级并入当前会话树。两种格式都不含可用的明文口令，导入的会话在连接时补凭据。第三方标识的来源与权利人见 NOTICE.md。
- **快速连接与认证**：Public Key / Password / Keyboard Interactive 三种方式，顺序即后端尝试顺序；私钥支持 OpenSSH 格式与 `~` 展开；勾选存储的口令在 Windows 上经 DPAPI 加密后落盘。
- **终端标签页**：真实 SSH PTY，自适应尺寸、多会话并行，标签显示连接状态；空闲保活在 SSH 层做。
- **命令库**：与会话树共用左侧停靠区。命令的显示名与实际发送文本分开存储，允许多行；可广播给全部就绪标签页或只发给当前可见的那个。
- **文件面板**：应用内右侧分栏，本地 + 远端双栏，上传 / 下载 / 新建目录 / 重命名 / 删除。
- **MCP 服务**：勾选后在 `127.0.0.1:39117/mcp`（端口可改）起一个 Streamable HTTP 的 MCP server，把已存会话作为工具开放给外部智能体（Qoder / Claude Desktop / Cursor），设置弹窗里一键复制可直接粘进客户端的 `{ mcpServers: { onecrt: … } }` JSON。六个工具：`sessions_list`、`terminals_list`、`exec`、`send`、`read`、`close`。`exec` 按会话复用同一个 shell，`cd` 与 `export` 跨调用有效，单次输出上限 64 KB、超出部分给出续读偏移。默认关闭、只监听回环、逐请求校验 `Authorization: Bearer <token>`；智能体只能点用应用已存的凭据，既收不到也看不到口令本身。
- **深浅两套主题**：配色 token 集中在 `src/styles.css`，界面与终端读同一组 CSS 变量，切换时终端前景/背景/光标/选区/ANSI 色一起跟上。

## 开发

```bash
npm install
npm run tauri dev      # 桌面端
npm run tauri build    # 打包
```

Rust 侧 `russh` 关掉默认特性、用 `ring` 作加密后端（`aws-lc-rs` 在 Windows 上要 NASM）。

## 测试

```bash
cd src-tauri && cargo test
```

- `tests/ssh_e2e.rs`：同进程起一个 russh 服务端，走真实的连接 → 认证 → PTY/shell → 输入输出 → 调整窗口 → 关闭，并覆盖密码错误与主机不可达两条失败路径。
- `tests/mcp_http.rs`：用 rmcp 自带客户端向本机临时端口做一次真实握手（配置写在 `%TEMP%`，端口填 0 由内核挑）——令牌不对时中途被 401 拦掉，对了则列全六个工具并调用一个无副作用的 `terminals_list`；另两条守启动状态机：重新生成令牌后仍要在同一端口监听且旧令牌立刻失效，按原端口重绑不能撞 `os error 10048`。
- `src/mcp.rs` 的单元测试锁 agent 输出缓冲的偏移语义：`read_from` 只回 from 之后的增量，只有 from 落在被挤掉的前缀里才报丢弃。

`.github/workflows/ci.yml` 是每次 push / PR 的门：`tsc` + `vite build` + 中英文案词典核对 + `cargo check --all-targets` + `cargo test`。

## 发布与自动更新

打 tag `v*` 触发 `.github/workflows/release.yml`：三平台并行构建，产物汇总成一条 GitHub Release，正文是带大小与 SHA256 的下载表加本次提交清单；手动 `workflow_dispatch` 只出 workflow 产物、不发版。版本号只认 `package.json`，`src-tauri/tauri.conf.json` 与 `src-tauri/Cargo.toml` 必须与它一致（meta job 交叉校验，不一致直接失败）。

应用内更新（帮助 → 关于 → 检查更新）读同一 Release 的 `latest.json`，安装包由 minisign 分离签名。一次性配置：仓库 Secret 加 `TAURI_SIGNING_PRIVATE_KEY`（`npx tauri signer generate --ci -w ~/.tauri/shellx_updater.key` 生成的私钥内容），公钥写进 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`——私钥丢了，已发出去的客户端就再也收不到更新。

Windows 与 Linux（AppImage）可完整自助升级；macOS 的自动更新按 Tauri 要求需要 Developer ID 签名，未签名包请手动下 dmg。

## 数据位置

会话、命令与 MCP 服务的开关/端口/令牌保存在 `dirs::config_dir()/shellx/`（Windows：`%APPDATA%\shellx\`）下的 `sessions.json`、`commands.json`、`mcp.json`。MCP 令牌明文存放；改端口会重启监听，重新生成令牌不会（鉴权逐请求现读，旧令牌当场失效）。口令在 Windows 上经 DPAPI 加密后落盘（`dpapi:` 前缀的十六进制，密钥由当前登录用户持有），Linux 与 macOS 没有可移植的系统级密钥、口令明文存放；私钥路径始终明文。

产品名叫 OneCRT（仓库 `onecrt`），但三处历史标识按原名保留：应用标识 `com.sprin.shellx`、上面这个数据目录、以及 `localStorage` 里的 `shellx.*` 键（主题、布局、停靠记忆）。它们都是已装客户端正在读的持久化位置，改了等于把老用户的会话和界面状态原地清空。

## 已知取舍

- 界面按同类终端的形态还原，但**只实现 SSH2**：协议下拉里的 SSH1 / Telnet / Telnet TLS / Local Shell / RDP / Serial / TAPI / Rlogin / Raw 一律置灰。
- 首次连接自动接受服务器主机密钥，不做指纹确认。
- 菜单与右键列全条目，其中打印、键盘映射、端口转发、标签组、多窗口、SFTP 会话一类是置灰占位，尚未实现。
- 不做断线自动重连：断线只给提示，按回车重连；空闲掉线靠 SSH 层保活压。
- MCP 只覆盖 SSH 终端这一层：没有把 SFTP 读写做成工具；agent 用的是无头会话（`mcp-` 前缀的 terminal id），不进布局、不出现在标签页，也看不到用户自己开着的标签；关掉开关会连带拆掉 agent 遗留的 shell。
- 本工具面向内网运维场景，不建议用在对外分发的机器上。
- 明亮主题下启动瞬间原生标题栏仍偏暗，要等前端 `setTheme` 生效才纠正（窗口底色已与深色 `--bg` 对齐，深色主题不再有跳变）。
