# OneCRT

跨平台 SSH 客户端，Tauri 2 + Rust（russh）+ React + xterm.js。界面采用 Session Manager / Quick Connect / Command Manager 布局。

## 功能

- **Session Manager**：文件夹 + 会话树，支持过滤、新建文件夹、删除、双击连接，树结构持久化为 JSON。
- **导入外部配置**：文件 → 导入配置… 打开来源弹窗（SecureCRT、WinSCP，各带官方标识），选中来源后在系统文件框里挑它的配置文件（SecureCRT 导出的会话 XML / WinSCP 的 WinSCP.ini），解析出的会话连同分组层级追加到当前会话树。两种配置都不含可用的明文口令，导入的会话在连接时补凭据。第三方标识的来源与权利人见 NOTICE.md。
- **会话树右键菜单**：Connect Terminal、Cut / Copy / Paste（树节点级，粘贴到右键目标所在文件夹）、Find、Delete、Rename、Expand / Collapse All Folders、Properties。
- **Quick Connect**：Protocol / Hostname / Port / Username / 认证方式（PublicKey、Keyboard Interactive、Password、GSSAPI），认证属性（口令、私钥路径）默认展开，可保存为会话节点。
- **终端标签页**：xterm.js 渲染，真实 SSH PTY，支持自适应尺寸、粘贴、多会话并行，标签上显示连接状态。
- **标签页右键菜单**：Rename / Reset Name、Reconnect / Disconnect、Close (Ctrl+F4) / Close Disconnected Tabs / Close Other Tabs / Close Tabs to the Right / Close All Tabs、Clone Session、Session Options...。Rename 只改标签显示名，Reset Name 回到会话名；改会话本身用 Session Options。Clone Session 用同一份连接参数另开一个标签页，快速连接型标签（无对应会话节点）的 Session Options 置灰。
- **Command Manager**：与 Session Manager 共用左侧停靠区，底部两个标签切换。同样是文件夹 + 命令树，支持过滤、新建命令 / 新建文件夹、剪切 / 复制 / 粘贴、删除、重命名、展开 / 折叠全部、属性；双击命令或点工具栏箭头发送到当前终端；右键 "Send Command to" 按作用域发送：All Sessions 广播给所有就绪标签页，Visible Sessions 只发给当前可见的那个。命令的显示名与实际发送文本分开存储，允许多行。
- **工具栏**：菜单栏下方一行，依次为：新建连接（用活动标签的连接参数打开 Quick Connect，没有活动标签时是空白新建）、切换下一个标签、重新连接 | 复制、粘贴、打印 | 会话选项、键盘映射、端口转发。有活动标签时才可用；打印 / 键盘映射 / 端口转发为置灰占位。视图菜单的"工具栏"控制这一行。
- **关于对话框**：帮助 → 关于 OneCRT… 给出版本、运行平台与数据目录（「打开」直接在文件管理器里定位该目录），四个外链（项目主页 / Releases 与下载 / 提交 issue / 第三方标识声明 NOTICE）与依赖致谢；「复制版本信息」一次拷走版本、平台、数据目录，提 issue 时直接粘。外链与打开目录都交回系统默认应用（`tauri-plugin-opener`）。
- **认证**：密码、公钥（OpenSSH 私钥，支持 `~` 展开）、键盘交互；失败时回传具体原因。
- **MCP 服务**：文件 → MCP 服务… 打开设置弹窗，勾选后在 `127.0.0.1:39117/mcp`（端口可改）起一个 Streamable HTTP 的 MCP server，把已存会话作为工具开放给外部智能体（Qoder / Claude Desktop / Cursor），弹窗里的复制按钮给出可直接粘进客户端的 `{ mcpServers: { onecrt: … } }` JSON。工具六个：`sessions_list`、`terminals_list`、`exec`、`send`、`read`、`close`。`exec` 按会话复用同一个 shell，`cd`、`export` 跨调用有效；命令是否跑完由追加的 `echo` 结束标记判定，回传正文带退出码，输出单次上限 64 KB，超出部分给出续读偏移。默认关闭、只监听回环、每个请求校验 `Authorization: Bearer <token>`；智能体只能点用 OneCRT 已存的凭据，既收不到也看不到口令本身。弹窗两秒刷新一次，列出当前在跑的 agent 会话与它们最后一条命令。

## 开发

```bash
npm install
npm run tauri dev      # 桌面端
npm run tauri build    # 打包
```

Rust 侧 `russh` 关闭了默认特性并使用 `ring` 作为加密后端（`aws-lc-rs` 在 Windows 上需要 NASM）。

## 发布

打 tag `v*` 触发 `.github/workflows/release.yml`：Windows（NSIS）、Linux（deb + AppImage）、macOS Apple Silicon（dmg + app.tar.gz）三个 job 并行构建，产物汇总成一条 GitHub Release，正文是带大小与 SHA256 的下载表加本次提交清单。手动 `workflow_dispatch` 只出 workflow 产物、不发版，用来试跑管线。

版本号只认 `package.json`，`src-tauri/tauri.conf.json` 与 `src-tauri/Cargo.toml` 必须与它一致（meta job 交叉校验，不一致直接失败）。

应用内更新（帮助 → 关于 → 检查更新）读同一 Release 的 `latest.json`，安装包由 minisign 分离签名。一次性配置：仓库 Secret 加 `TAURI_SIGNING_PRIVATE_KEY`（`npx tauri signer generate --ci -w ~/.tauri/shellx_updater.key` 生成的私钥内容），公钥写进 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`；私钥丢了旧客户端就再也收不到更新。本地要出签名包：

```bash
TAURI_SIGNING_PRIVATE_KEY=~/.tauri/shellx_updater.key npm run tauri build -- --config '{"bundle":{"createUpdaterArtifacts":true}}'
```

Windows 与 Linux(AppImage) 可完整自助升级；macOS 的自动更新按 Tauri 要求需 Developer ID 签名，未签名的包请手动下载 dmg。`.github/workflows/ci.yml` 是每次 push / PR 的门：`tsc` + `vite build` + 中英文案词典核对 + `cargo check` + `cargo test`。

## 外观

支持深色 / 明亮两套主题，侧栏工具条最右侧的日/月图标按钮切换，选择记在 `localStorage`（`shellx.theme`）；原生标题栏随 `window.setTheme` 同步。两套色值都按参考截图逐像素取样得到，同属冷调（蓝灰底、不带米色），深色是「外框亮、内容层暗」，明亮是「外框暗、内容层纯白」。配色 token 集中在 `src/styles.css`：`:root` 是深色，`[data-theme="light"]` 覆盖同名变量（`--bg` / `--bg-content` / `--bg-selected` / `--border` / `--text*` / `--accent` / `--scrim` / `--shadow-*` 等），加新主题只动这一处。xterm 不再单独写死色值，`src/Terminal.tsx` 从同一组 CSS 变量读背景/前景/光标/选区与 ANSI 色，切主题时实时更新，两边背景天然一致。

文字分三档，按角色而非按参考图的位置套用：`--text` 给面板主内容（会话树、面板标题、对话框标签与选项），`--text-2` 给未选中标签、工具条图标、空态提示，`--text-3` 只留给占位符、箭头/关闭等小图标、快捷键列与禁用项。参考图里文件树用的是次级灰，但会话树是本应用左栏的主内容，所以这里升到 `--text`（深色两档实测：树标签 #A2A5A7 → #E7E9ED，明亮 #727985 → #2B3547）。

实测对比度（正文 `--text` / 次级 `--text-2` / 强调绿，各自对内容层底色）：深色 14.4 / 7.1 / 7.1，明亮 12.3 / 4.4 / 2.1。绿色在两档主题里都只作填充与描边（`accent-color`、边框、开关、终端光标），没有用作文字色，所以明亮档的 2.1 不影响可读性；主按钮是近黑标签压绿底，8.4（明亮）/ 7.2（深色）。`--text-3` 那档 2.5（明亮）/ 3.9（深色），按参考图原样保留。

尺寸与字阶也按参考图量：列表行高 22px（参考图选中条 44 物理像素 @200%，实测本应用改后同为 44）、侧栏标题 11px、正文与标签 13px、说明文字 12px。列表选中另设 `--bg-active`，两档主题都分「胶囊」和「整行」两级（深档实测 `#3E3F41` / `#46484A`，亮档 `#E4E8EF` / `#DDE3EC`）。终端的 ANSI 红/黄/绿从 `--ansi-*` 读，不再吃 xterm 默认值——默认红在 `#181a1b` 上实测只有 1.9:1，中文报错看不清，改后红/黄/绿为 4.9 / 9.6 / 7.1（深）与 6.1 / 5.1 / 6.2（亮）。

亮档第二轮按参考图再量一次：面板分隔线 `#DDDDDD`（2 物理像素竖线）、浮层底 `#F8F8F8`、输入框描边与规则线 `#CDD2DC`、侧栏头下那道 6 像素渐变分界的深端 `#EAEBEC`，逐一对应 `--border` / `--menu-bg` / `--border-strong` / `--line`。原来的选中档 `#EDEFF2` 相对外框只有 −0.069 线性亮度，在 `#f6f7fa` 上几乎看不出来，压深到 −0.126 / −0.167 两级后正文仍有 10.0 / 9.6:1。另一处结构问题：没有标签时编辑区落在外框档，整片发灰，`.panes` 补上 `--bg-content` 后空态也是纯白内容层——实跑截图按精确色直方图核对，白区 2,707,230 像素对 `#F6F7FA` 外框 1,343,346 像素。

## 测试

```bash
cd src-tauri && cargo test
```

`tests/ssh_e2e.rs` 在同进程内起一个 russh 服务端，走真实的连接 → 认证 → PTY/shell → 输入输出 → 调整窗口大小 → 关闭链路，并覆盖密码错误与主机不可达两条失败路径。

`tests/mcp_http.rs` 用 rmcp 自带的客户端向本机临时端口连一次真实握手（配置写在 `%TEMP%` 的临时 `mcp.json` 里，端口填 0 由内核挑，不碰 `%APPDATA%`）：不带正确令牌时握手中途被 401 拦掉，带令牌时列全六个工具并成功调用一个无副作用的 `terminals_list`（单线程与多线程 runtime 各跑一遍）。另两条守启动状态机：重新生成令牌后仍要在同一端口监听、旧令牌立刻 401；按原端口重绑不能撞 `os error 10048`（监听任务没真退出就重绑）。`src/mcp.rs` 里的单元测试锁 agent 输出缓冲的偏移语义：`read_from` 只回 from 之后的增量、只有 from 落在被挤掉的前缀里才报丢弃。

## 数据位置

会话与命令保存在 `dirs::config_dir()/shellx/`（Windows：`%APPDATA%\shellx\`）下的 `sessions.json`、`commands.json`。MCP 服务的开关、端口与令牌在同目录的 `mcp.json`，令牌明文存放；改端口会重启监听，重新生成令牌不会（鉴权逐请求现读，客户端下次请求就用新值，旧令牌当场失效）。口令在 Windows 上经 DPAPI 加密后落盘（存成 `dpapi:` 前缀的十六进制，密钥由当前登录用户持有），私钥路径明文；Linux 与 macOS 没有可移植的系统级密钥，口令明文存放。仅面向内网运维场景，请勿用于对外分发的机器。

产品名叫 OneCRT（仓库 `onecrt`），但三处历史标识按原名保留：应用标识 `com.sprin.shellx`、上面那个数据目录、以及 `localStorage` 里的 `shellx.*` 键（主题、布局、停靠记忆）。它们都是已装客户端正在用的持久化位置，改了等于把老用户的会话和界面状态原地清空，所以不跟随改名。

## 已知取舍

- 首次连接自动接受服务器主机密钥，不做指纹确认。
- 协议下拉列出了 SSH2 / SSH1 / Telnet / Telnet/TLS / Local Shell / RDP / Serial / TAPI / Rlogin / Raw，但只有 **SSH2** 已实现，其余置灰不可选。
- Firewall 选项固定为 None；"Show quick connect on startup" 与 "Open in a tab" 两个勾选框当前仅为界面还原，不改变行为。
- 右键菜单列全条目，其中 Connect in New Window / New Tab Group / File Transfer / Both、Find Next、Create Desktop Shortcut、Arrange Tree、Show Descriptions、Show Transfer Sessions、Open Sessions in a Tab/Tile、文件夹 Properties 均为置灰占位，尚未实现。命令树的右键菜单已实现，其中 "Send Command to" 的 Selected Tab Group(s) 置灰占位（还没有标签组），命令文件夹本身也不能整组批量发送。
- 菜单右侧的快捷键（Ctrl+Alt+T 等）当前只作对照显示，未绑定全局按键；菜单内可用 ↑ ↓ Enter Esc 操作，→ 打开子菜单、← 关闭。
- 标签页右键菜单同样列全条目，其中 Close Tab Group / Lock Session... / Clone in New Window / Send to New Window / Send to New Tab Group / Connect SFTP Session / Open SecureFX / Save Session... / Send Commands to This Group 为置灰占位：它们分别依赖标签组、多窗口与 SFTP，这三样都还没实现。
- 命令树面板没有还原标题栏右侧的置顶 / 关闭两个小图标，停靠区改由底部的 Session Manager / Command Manager 标签切换。
- MCP 服务只覆盖 SSH 终端这一层：没有把 SFTP 文件读写做成工具，agent 用无头会话（`mcp-` 前缀的 terminal id）连进去，这些会话不进 flexlayout 布局、不出现在标签页里，也就看不到用户自己开着的标签；关闭 MCP 开关会连带拆掉 agent 遗留的 shell。服务默认关闭，`mcp.json` 里 `enabled: true` 才会在启动时拉起；端口被占用时开关会自动落回关闭并在弹窗里显示失败原因。
- 主题只覆盖界面配色：`tauri.conf.json` 里窗口的 `theme: "Dark"` 是写死的，明亮主题下启动瞬间标题栏仍偏暗，要等前端 `setTheme` 生效才纠正（`backgroundColor` 已与深色 `--bg` 对齐为 `#222426`，深色主题不再有底色跳变）。终端 ANSI 只接了红/黄/绿三档（`--ansi-*`，报错与断连提示走的就是这三档），其余 13 色仍用 xterm 默认调色板。