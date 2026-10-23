# ShellX

跨平台 SSH 客户端，Tauri 2 + Rust（russh）+ React + xterm.js。界面采用 Session Manager / Quick Connect / Command Manager 布局。

## 功能

- **Session Manager**：文件夹 + 会话树，支持过滤、新建文件夹、删除、双击连接，树结构持久化为 JSON。
- **会话树右键菜单**：Connect Terminal、Cut / Copy / Paste（树节点级，粘贴到右键目标所在文件夹）、Find、Delete、Rename、Expand / Collapse All Folders、Properties。
- **Quick Connect**：Protocol / Hostname / Port / Username / 认证方式（PublicKey、Keyboard Interactive、Password、GSSAPI），认证属性（口令、私钥路径）默认展开，可保存为会话节点。
- **终端标签页**：xterm.js 渲染，真实 SSH PTY，支持自适应尺寸、粘贴、多会话并行，标签上显示连接状态。
- **标签页右键菜单**：Rename / Reset Name、Reconnect / Disconnect、Close (Ctrl+F4) / Close Disconnected Tabs / Close Other Tabs / Close Tabs to the Right / Close All Tabs、Clone Session、Session Options...。Rename 只改标签显示名，Reset Name 回到会话名；改会话本身用 Session Options。Clone Session 用同一份连接参数另开一个标签页，快速连接型标签（无对应会话节点）的 Session Options 置灰。
- **Command Manager**：与 Session Manager 共用左侧停靠区，底部两个标签切换。同样是文件夹 + 命令树，支持过滤、新建命令 / 新建文件夹、剪切 / 复制 / 粘贴、删除、重命名、展开 / 折叠全部、属性；双击命令或点工具栏箭头发送到当前终端；右键 "Send Command to" 按作用域发送：All Sessions 广播给所有就绪标签页，Visible Sessions 只发给当前可见的那个。命令的显示名与实际发送文本分开存储，允许多行。
- **工具栏**：菜单栏下方一行，依次为：新建连接（用活动标签的连接参数打开 Quick Connect，没有活动标签时是空白新建）、切换下一个标签、重新连接 | 复制、粘贴、打印 | 会话选项、键盘映射、端口转发。有活动标签时才可用；打印 / 键盘映射 / 端口转发为置灰占位。视图菜单的"工具栏"控制这一行。
- **认证**：密码、公钥（OpenSSH 私钥，支持 `~` 展开）、键盘交互；失败时回传具体原因。

## 开发

```bash
npm install
npm run tauri dev      # 桌面端
npm run tauri build    # 打包
```

Rust 侧 `russh` 关闭了默认特性并使用 `ring` 作为加密后端（`aws-lc-rs` 在 Windows 上需要 NASM）。

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

## 数据位置

会话与命令保存在 `dirs::config_dir()/shellx/`（Windows：`%APPDATA%\shellx\`）下的 `sessions.json`、`commands.json`。密码与私钥路径明文存放，仅面向内网运维场景，请勿用于对外分发的机器。

## 已知取舍

- 首次连接自动接受服务器主机密钥，不做指纹确认。
- 协议下拉列出了 SSH2 / SSH1 / Telnet / Telnet/TLS / Local Shell / RDP / Serial / TAPI / Rlogin / Raw，但只有 **SSH2** 已实现，其余置灰不可选。
- Firewall 选项固定为 None；"Show quick connect on startup" 与 "Open in a tab" 两个勾选框当前仅为界面还原，不改变行为。
- 右键菜单列全条目，其中 Connect in New Window / New Tab Group / File Transfer / Both、Find Next、Create Desktop Shortcut、Arrange Tree、Show Descriptions、Show Transfer Sessions、Open Sessions in a Tab/Tile、文件夹 Properties 均为置灰占位，尚未实现。命令树的右键菜单已实现，其中 "Send Command to" 的 Selected Tab Group(s) 置灰占位（还没有标签组），命令文件夹本身也不能整组批量发送。
- 菜单右侧的快捷键（Ctrl+Alt+T 等）当前只作对照显示，未绑定全局按键；菜单内可用 ↑ ↓ Enter Esc 操作，→ 打开子菜单、← 关闭。
- 标签页右键菜单同样列全条目，其中 Close Tab Group / Lock Session... / Clone in New Window / Send to New Window / Send to New Tab Group / Connect SFTP Session / Open SecureFX / Save Session... / Send Commands to This Group 为置灰占位：它们分别依赖标签组、多窗口与 SFTP，这三样都还没实现。
- 命令树面板没有还原标题栏右侧的置顶 / 关闭两个小图标，停靠区改由底部的 Session Manager / Command Manager 标签切换。
- 主题只覆盖界面配色：`tauri.conf.json` 里窗口的 `theme: "Dark"` 是写死的，明亮主题下启动瞬间标题栏仍偏暗，要等前端 `setTheme` 生效才纠正（`backgroundColor` 已与深色 `--bg` 对齐为 `#222426`，深色主题不再有底色跳变）。终端 ANSI 只接了红/黄/绿三档（`--ansi-*`，报错与断连提示走的就是这三档），其余 13 色仍用 xterm 默认调色板。