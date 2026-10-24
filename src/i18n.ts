import { useSyncExternalStore } from "react";

export type Locale = "zh" | "en";

const STORE_KEY = "shellx.lang";

type Dict = Record<string, string>;

const zh: Dict = {
  // 通用
  "common.ok": "确定",
  "common.cancel": "取消",
  "common.close": "关闭",
  "common.deleteConfirm": '确定删除 "{name}" 吗？',

  // 菜单栏
  "menu.file": "文件",
  "menu.file.newSession": "新建会话…",
  "menu.file.newFolder": "新建文件夹…",
  "menu.file.closeTab": "关闭当前标签",
  "menu.file.quit": "退出",
  "menu.edit": "编辑",
  "menu.edit.copy": "复制",
  "menu.edit.paste": "粘贴",
  "menu.edit.selectAll": "全选",
  "menu.terminal": "终端",
  "menu.terminal.reconnect": "重新连接",
  "menu.terminal.disconnect": "断开连接",
  "menu.terminal.sendCtrlC": "发送 Ctrl+C",
  "menu.terminal.clear": "清屏",
  "menu.view": "视图",
  "menu.view.menuBar": "菜单栏",
  "menu.view.toolbar": "工具栏",
  "menu.view.sessionTabs": "会话标签栏",
  "menu.view.statusBar": "状态栏",
  "menu.view.sessionManager": "会话管理器",
  "menu.view.cmdManager": "命令管理器",
  "menu.view.filePanel": "文件面板 (SFTP)",
  "menu.view.perfPanel": "性能监控",
  "menu.view.transfers": "传输列表",
  "menu.view.resetLayout": "重置窗口布局",
  "menu.view.zoomIn": "放大",
  "menu.view.zoomOut": "缩小",
  "menu.view.zoomReset": "重置缩放",
  "menu.view.darkTheme": "深色主题",
  "menu.view.fullscreen": "全屏",
  "menu.view.alwaysOnTop": "总在最前",
  "menu.view.language": "语言",
  "menu.help": "帮助",
  "menu.help.about": "关于 ShellX…",

  // 关于
  "about.title": "关于 ShellX",
  "about.desc": "跨平台 SSH 客户端：Tauri 2 + Rust（russh）+ React + xterm.js。",
  "about.note": "会话与命令保存在 %APPDATA%\\shellx\\，口令明文存储，仅面向内网运维。",
  "about.checkUpdate": "检查更新",
  "about.checking": "正在检查更新…",
  "about.latest": "当前已是最新版本",
  "about.installing": "正在下载并安装 {ver}，完成后自动重启",
  "about.error": "检查更新失败：无网络、仓库不可达或当前不是安装包运行的版本",

  // 状态栏
  "status.connecting": "连接中",
  "status.ready": "已连接",
  "status.error": "连接失败",
  "status.closed": "已断开",
  "status.noSession": "无活动会话",
  "status.tabs": "{n} 个标签",

  // 终端内提示
  "term.reconnectHint": "按回车重连",

  // 快速连接 / 会话属性
  "qc.defaultTitle": "快速连接",
  "qc.sessionPropsTitle": "会话属性 - {name}",
  "qc.protocol": "协议：",
  "qc.hint": "当前仅实现 SSH2，其余协议在列表中置灰占位。",
  "qc.sessionName": "会话名称：",
  "qc.hostname": "主机名：",
  "qc.port": "端口：",
  "qc.firewall": "防火墙：",
  "qc.none": "无",
  "qc.username": "用户名：",
  "qc.auth": "身份验证",
  "qc.properties": "属性…",
  "qc.keyPlaceholder": "C:\\Users\\me\\.ssh\\id_ed25519 或 ~\\.ssh\\id_rsa",
  "qc.showOnStartup": "启动时显示快速连接",
  "qc.saveSession": "保存会话",
  "qc.openInTab": "在标签页中打开",
  "qc.connect": "连接",
  "qc.moveUp": "上移",
  "qc.moveDown": "下移",
  "qc.authNote": "列表顺序即连接时的尝试顺序；口令与身份文件在各自方法的 Properties 里设置。",

  // 身份验证方法属性
  "ap.publicKeyTitle": "公钥属性",
  "ap.passwordTitle": "口令属性",
  "ap.kbdTitle": "键盘交互属性",
  "ap.genericTitle": "{method} 属性",
  "ap.identity": "身份",
  "ap.identityFile": "身份文件：",
  "ap.browse": "浏览",
  "ap.fingerprint": "指纹：",
  "ap.fpPlaceholder": "填入身份文件后显示 SHA256 指纹",
  "ap.password": "口令",
  "ap.storePassword": "保存口令",
  "ap.confirmPassword": "确认口令",
  "ap.mismatch": "两次输入的口令不一致",
  "ap.encryptedNote": "（口令将被加密保存。）",
  "ap.ok": "确定",
  "ap.cancel": "取消",

  // 口令对话框
  "pwd.title": "输入 Secure Shell 密码",
  "pwd.requires": "{user}@{host} 需要密码。",
  "pwd.enterNow": "请输入密码。",
  "pwd.username": "用户名：",
  "pwd.password": "密码：",
  "pwd.save": "保存密码",
  "pwd.ok": "确定",
  "pwd.cancel": "取消",
  "pwd.skip": "跳过",

  // 命令属性
  "cmd.propsTitle": "命令属性 - {name}",
  "cmd.newTitle": "新建命令",
  "cmd.name": "名称：",
  "cmd.command": "命令：",
  "cmd.textPlaceholder": "留空则使用名称作为命令文本",
  "cmd.ok": "确定",
  "cmd.cancel": "取消",

  // 确认框
  "confirm.title": "确认",

  // 图钉 / 浮窗
  "pin.unpinTitle": "自动隐藏（收进停靠栏）",
  "pin.pinTitle": "钉住（常驻停靠）",
  "pin.unpin": "自动隐藏",
  "pin.pin": "钉住",
  "float.dockBack": "回到停靠位置",

  // 工具栏
  "tb.newConnect": "新建连接",
  "tb.nextTab": "切换到下一个标签",
  "tb.reconnect": "重新连接",
  "tb.copy": "复制",
  "tb.paste": "粘贴",
  "tb.print": "打印",
  "tb.fileTransfer": "文件传输 / SFTP",
  "tb.sessionOptions": "会话选项",
  "tb.keyMapping": "键盘映射",
  "tb.portForward": "端口转发 / 隧道",

  // 标题栏
  "titlebar.minimize": "最小化",
  "titlebar.restore": "向下还原",
  "titlebar.maximize": "最大化",
  "titlebar.close": "关闭",

  // 传输面板
  "tp.title": "传输",
  "tp.clearFinished": "清除已完成的记录",
  "tp.close": "关闭传输面板",
  "tp.colFile": "文件",
  "tp.colProgress": "进度",
  "tp.colSize": "大小",
  "tp.colRate": "速率",
  "tp.colElapsed": "耗时",
  "tp.colStatus": "状态",
  "tp.status.running": "传输中",
  "tp.status.done": "完成",
  "tp.status.error": "失败",
  "tp.empty": "暂无传输：在文件面板里选中文件后点上传或下载",

  // 命令管理器
  "cm.title": "命令管理器",
  "cm.send": "发送命令",
  "cm.newCommand": "新建命令",
  "cm.newFolder": "新建文件夹",
  "cm.cut": "剪切",
  "cm.copy": "复制",
  "cm.paste": "粘贴",
  "cm.delete": "删除",
  "cm.properties": "属性",
  "cm.filter": "按文件夹/命令名筛选",

  // 会话管理器
  "sm.title": "会话管理器",
  "sm.newFolder": "新建文件夹",
  "sm.newSession": "新建会话",
  "sm.sessionProps": "会话属性",
  "sm.delete": "删除",
  "sm.toLight": "切换到明亮主题",
  "sm.toDark": "切换到深色主题",
  "sm.filter": "按文件夹/会话名筛选",

  // 认证方式（会话里存的是英文方法名，这里只是显示用）
  "auth.PublicKey": "公钥",
  "auth.Password": "密码",
  "auth.Keyboard Interactive": "键盘交互式",

  // 文件面板
  "fp.title": "文件面板",
  "fp.up": "上一级",
  "fp.refresh": "刷新",
  "fp.pathPlaceholder": "路径，回车跳转",
  "fp.empty": "空目录",
  "fp.connecting": "连接中…",
  "fp.notConnected": "未连接",
  "fp.noActiveTab": "没有活动标签页",
  "fp.upload": "上传：本地选中项 → 远端当前目录",
  "fp.download": "下载：远端选中项 → 本地当前目录",
  "fp.newDirLocal": "在本地当前目录新建子目录",
  "fp.newDirRemote": "在远端当前目录新建子目录",
  "fp.rename": "重命名选中项",
  "fp.delete": "删除选中项",
  "fp.close": "关闭文件面板",
  "fp.local": "本地",
  "fp.remote": "远端",
  "fp.home": "主目录",
  "fp.quickAccess": "快速访问：常用文件夹和驱动器",
  "fp.serverRoot": "服务器根目录",
  "fp.selected": "已选中 {name} · {side}",
  "fp.footHint": "{conn} · 点选一侧后使用新建/重命名/删除",
  "fp.sftpError": "SFTP 连接失败：{e}",

  // 空态
  "app.emptyTitle": "暂无终端会话",
  "app.emptyDescHas": "双击左侧会话即可连接",
  "app.emptyDescNone": "还没有保存的会话",
  "app.emptyNew": "新建连接",

  // flexlayout 内建串
  "fl.dockFloatToLayout": "拖回主窗口停靠",
  "fl.dockFloatTabs": "把 ? 个标签拖回主窗口",

  // 右键菜单：会话树
  "ctx.connectTerminal": "连接终端",
  "ctx.connectNewWindow": "在新窗口中连接",
  "ctx.connectNewTabGroup": "在新标签组中连接",
  "ctx.connectFileTransfer": "连接文件传输",
  "ctx.connectBoth": "同时连接",
  "ctx.cut": "剪切",
  "ctx.copy": "复制",
  "ctx.paste": "粘贴",
  "ctx.find": "查找…",
  "ctx.findNext": "查找下一个",
  "ctx.delete": "删除",
  "ctx.rename": "重命名",
  "ctx.createShortcut": "创建桌面快捷方式",
  "ctx.expandAll": "展开全部文件夹",
  "ctx.collapseAll": "折叠全部文件夹",
  "ctx.arrangeTree": "排列树",
  "ctx.showDescriptions": "显示描述",
  "ctx.showTransferSessions": "显示传输会话",
  "ctx.openSessionsInTab": "在标签/平铺中打开会话",
  "ctx.properties": "属性",

  // 右键菜单：命令树
  "ctx.sendCommand": "发送命令",
  "ctx.sendCommandTo": "发送命令到",
  "ctx.allSessions": "所有会话",
  "ctx.selectedTabGroups": "选定的标签组",
  "ctx.visibleSessions": "可见会话",

  // 右键菜单：标签页
  "ctx.resetName": "重置名称",
  "ctx.reconnect": "重新连接",
  "ctx.disconnect": "断开连接",
  "ctx.close": "关闭",
  "ctx.closeDisconnected": "关闭已断开的标签",
  "ctx.closeOther": "关闭其他标签",
  "ctx.closeRight": "关闭右侧标签",
  "ctx.closeTabGroup": "关闭标签组",
  "ctx.closeAll": "关闭所有标签",
  "ctx.lockSession": "锁定会话…",
  "ctx.cloneSession": "克隆会话",
  "ctx.cloneNewWindow": "在新窗口中克隆",
  "ctx.sendToNewWindow": "发送到新窗口",
  "ctx.sendToNewTabGroup": "发送到新标签组",
  "ctx.connectSftp": "连接 SFTP 会话",
  "ctx.openSecureFX": "打开 SecureFX",
  "ctx.sessionOptions": "会话选项…",
  "ctx.saveSession": "保存会话…",
  "ctx.sendCommandsToGroup": "向此组发送命令",

  // 性能面板
  "perf.title": "性能",
  "perf.close": "关闭性能面板",
  "perf.intervalTip": "采样间隔",
  "perf.intervalOpt": "{s} 秒",
  "perf.noTab": "没有活动标签页：先连上一台服务器",
  "perf.connecting": "连接中…",
  "perf.waiting": "采集中，等第二个采样点",
  "perf.failed": "采样失败",
  "perf.noData": "目标机没有回传采样：需要 Linux 的 /proc 与 sh",
  "perf.cpu": "CPU",
  "perf.mem": "内存",
  "perf.net": "网络",
  "perf.disk": "磁盘 /",
  "perf.loadTip": "负载",
  "perf.memUsedShort": "已用",
  "perf.netSub": "下行 / 上行",
  "perf.peak": "峰值",
  "perf.diskFree": "可用",
  "perf.up": "运行",
  "perf.samples": "个采样",
  "perf.footSource": "数据取自目标机的 /proc 与 df",
};

const en: Dict = {
  "common.ok": "OK",
  "common.cancel": "Cancel",
  "common.close": "Close",
  "common.deleteConfirm": 'Delete "{name}"?',

  "menu.file": "File",
  "menu.file.newSession": "New Session…",
  "menu.file.newFolder": "New Folder…",
  "menu.file.closeTab": "Close Current Tab",
  "menu.file.quit": "Exit",
  "menu.edit": "Edit",
  "menu.edit.copy": "Copy",
  "menu.edit.paste": "Paste",
  "menu.edit.selectAll": "Select All",
  "menu.terminal": "Terminal",
  "menu.terminal.reconnect": "Reconnect",
  "menu.terminal.disconnect": "Disconnect",
  "menu.terminal.sendCtrlC": "Send Ctrl+C",
  "menu.terminal.clear": "Clear Screen",
  "menu.view": "View",
  "menu.view.menuBar": "Menu Bar",
  "menu.view.toolbar": "Toolbar",
  "menu.view.sessionTabs": "Session Tabs",
  "menu.view.statusBar": "Status Bar",
  "menu.view.sessionManager": "Session Manager",
  "menu.view.cmdManager": "Command Manager",
  "menu.view.filePanel": "File Panel (SFTP)",
  "menu.view.perfPanel": "Performance",
  "menu.view.transfers": "Transfer List",
  "menu.view.resetLayout": "Reset Window Layout",
  "menu.view.zoomIn": "Zoom In",
  "menu.view.zoomOut": "Zoom Out",
  "menu.view.zoomReset": "Reset Zoom",
  "menu.view.darkTheme": "Dark Theme",
  "menu.view.fullscreen": "Full Screen",
  "menu.view.alwaysOnTop": "Always On Top",
  "menu.view.language": "Language",
  "menu.help": "Help",
  "menu.help.about": "About ShellX…",

  "about.title": "About ShellX",
  "about.desc": "Cross-platform SSH client: Tauri 2 + Rust (russh) + React + xterm.js.",
  "about.note": "Sessions and commands are stored in %APPDATA%\\shellx\\; passwords in plaintext, for intranet ops use only.",
  "about.checkUpdate": "Check for updates",
  "about.checking": "Checking for updates…",
  "about.latest": "You're on the latest version",
  "about.installing": "Downloading and installing {ver}, the app will restart when done",
  "about.error": "Update check failed: no network, repository unreachable, or not running from an installed build",

  "status.connecting": "Connecting",
  "status.ready": "Connected",
  "status.error": "Connection failed",
  "status.closed": "Disconnected",
  "status.noSession": "No active session",
  "status.tabs": "{n} tabs",

  "term.reconnectHint": "Press Enter to reconnect",

  "qc.defaultTitle": "Quick Connect",
  "qc.sessionPropsTitle": "Session Properties - {name}",
  "qc.protocol": "Protocol:",
  "qc.hint": "Only SSH2 is implemented; other protocols are greyed out as placeholders.",
  "qc.sessionName": "Session name:",
  "qc.hostname": "Hostname:",
  "qc.port": "Port:",
  "qc.firewall": "Firewall:",
  "qc.none": "None",
  "qc.username": "Username:",
  "qc.auth": "Authentication",
  "qc.properties": "Properties...",
  "qc.keyPlaceholder": "C:\\Users\\me\\.ssh\\id_ed25519 or ~\\.ssh\\id_rsa",
  "qc.showOnStartup": "Show quick connect on startup",
  "qc.saveSession": "Save session",
  "qc.openInTab": "Open in a tab",
  "qc.connect": "Connect",
  "qc.moveUp": "Move up",
  "qc.moveDown": "Move down",
  "qc.authNote": "The list order is the order methods are tried when connecting; set the password and identity file in each method's Properties.",

  "ap.publicKeyTitle": "Public Key Properties",
  "ap.passwordTitle": "Password Properties",
  "ap.kbdTitle": "Keyboard Interactive Properties",
  "ap.genericTitle": "{method} Properties",
  "ap.identity": "Identity",
  "ap.identityFile": "Identity file:",
  "ap.browse": "Browse",
  "ap.fingerprint": "Fingerprint:",
  "ap.fpPlaceholder": "Shows the SHA256 fingerprint once an identity file is set",
  "ap.password": "Password",
  "ap.storePassword": "Store password",
  "ap.confirmPassword": "Confirm password",
  "ap.mismatch": "The two passwords do not match",
  "ap.encryptedNote": "(Password will be encrypted.)",
  "ap.ok": "OK",
  "ap.cancel": "Cancel",

  "pwd.title": "Enter Secure Shell Password",
  "pwd.requires": "{user}@{host} requires a password.",
  "pwd.enterNow": "Please enter a password now.",
  "pwd.username": "Username:",
  "pwd.password": "Password:",
  "pwd.save": "Save password",
  "pwd.ok": "OK",
  "pwd.cancel": "Cancel",
  "pwd.skip": "Skip",

  "cmd.propsTitle": "Command Properties - {name}",
  "cmd.newTitle": "New Command",
  "cmd.name": "Name:",
  "cmd.command": "Command:",
  "cmd.textPlaceholder": "Leave empty to use the name as the command text",
  "cmd.ok": "OK",
  "cmd.cancel": "Cancel",

  "confirm.title": "Confirm",

  "pin.unpinTitle": "Auto-hide (collapse to dock bar)",
  "pin.pinTitle": "Pin (keep docked)",
  "pin.unpin": "Auto-hide",
  "pin.pin": "Pin",
  "float.dockBack": "Dock back",

  "tb.newConnect": "New Connection",
  "tb.nextTab": "Next Tab",
  "tb.reconnect": "Reconnect",
  "tb.copy": "Copy",
  "tb.paste": "Paste",
  "tb.print": "Print",
  "tb.fileTransfer": "File Transfer / SFTP",
  "tb.sessionOptions": "Session Options",
  "tb.keyMapping": "Key Mapping",
  "tb.portForward": "Port Forwarding / Tunneling",

  "titlebar.minimize": "Minimize",
  "titlebar.restore": "Restore",
  "titlebar.maximize": "Maximize",
  "titlebar.close": "Close",

  "tp.title": "Transfers",
  "tp.clearFinished": "Clear finished",
  "tp.close": "Close transfer panel",
  "tp.colFile": "File",
  "tp.colProgress": "Progress",
  "tp.colSize": "Size",
  "tp.colRate": "Rate",
  "tp.colElapsed": "Elapsed",
  "tp.colStatus": "Status",
  "tp.status.running": "Transferring",
  "tp.status.done": "Done",
  "tp.status.error": "Failed",
  "tp.empty": "No transfers yet: select a file in the file panel and click upload or download",

  "cm.title": "Command Manager",
  "cm.send": "Send command",
  "cm.newCommand": "New command",
  "cm.newFolder": "New folder",
  "cm.cut": "Cut",
  "cm.copy": "Copy",
  "cm.paste": "Paste",
  "cm.delete": "Delete",
  "cm.properties": "Properties",
  "cm.filter": "Filter by folder/command name",

  "sm.title": "Session Manager",
  "sm.newFolder": "New folder",
  "sm.newSession": "New session",
  "sm.sessionProps": "Session properties",
  "sm.delete": "Delete",
  "sm.toLight": "Switch to light theme",
  "sm.toDark": "Switch to dark theme",
  "sm.filter": "Filter by folder/session name",

  "auth.PublicKey": "Public Key",
  "auth.Password": "Password",
  "auth.Keyboard Interactive": "Keyboard Interactive",

  "fp.title": "File Panel",
  "fp.up": "Up one level",
  "fp.refresh": "Refresh",
  "fp.pathPlaceholder": "Path, press Enter to go",
  "fp.empty": "Empty directory",
  "fp.connecting": "Connecting…",
  "fp.notConnected": "Not connected",
  "fp.noActiveTab": "No active tab",
  "fp.upload": "Upload: local selection → remote current directory",
  "fp.download": "Download: remote selection → local current directory",
  "fp.newDirLocal": "New subdirectory in the local current directory",
  "fp.newDirRemote": "New subdirectory in the remote current directory",
  "fp.rename": "Rename selection",
  "fp.delete": "Delete selection",
  "fp.close": "Close file panel",
  "fp.local": "Local",
  "fp.remote": "Remote",
  "fp.home": "Home directory",
  "fp.quickAccess": "Quick access: common folders and drives",
  "fp.serverRoot": "Server root",
  "fp.selected": "Selected {name} · {side}",
  "fp.footHint": "{conn} · select a side to use new/rename/delete",
  "fp.sftpError": "SFTP connection failed: {e}",

  "app.emptyTitle": "No terminal sessions",
  "app.emptyDescHas": "Double-click a session on the left to connect",
  "app.emptyDescNone": "No saved sessions yet",
  "app.emptyNew": "New connection",

  "fl.dockFloatToLayout": "Dock back to main window",
  "fl.dockFloatTabs": "Dock ? tabs back to main window",

  "ctx.connectTerminal": "Connect Terminal",
  "ctx.connectNewWindow": "Connect in New Window",
  "ctx.connectNewTabGroup": "Connect in New Tab Group",
  "ctx.connectFileTransfer": "Connect File Transfer",
  "ctx.connectBoth": "Connect Both",
  "ctx.cut": "Cut",
  "ctx.copy": "Copy",
  "ctx.paste": "Paste",
  "ctx.find": "Find...",
  "ctx.findNext": "Find Next",
  "ctx.delete": "Delete",
  "ctx.rename": "Rename",
  "ctx.createShortcut": "Create Desktop Shortcut",
  "ctx.expandAll": "Expand All Folders",
  "ctx.collapseAll": "Collapse All Folders",
  "ctx.arrangeTree": "Arrange Tree",
  "ctx.showDescriptions": "Show Descriptions",
  "ctx.showTransferSessions": "Show Transfer Sessions",
  "ctx.openSessionsInTab": "Open Sessions in a Tab/Tile",
  "ctx.properties": "Properties",

  "ctx.sendCommand": "Send Command",
  "ctx.sendCommandTo": "Send Command to",
  "ctx.allSessions": "All Sessions",
  "ctx.selectedTabGroups": "Selected Tab Group(s)",
  "ctx.visibleSessions": "Visible Sessions",

  "ctx.resetName": "Reset Name",
  "ctx.reconnect": "Reconnect",
  "ctx.disconnect": "Disconnect",
  "ctx.close": "Close",
  "ctx.closeDisconnected": "Close Disconnected Tabs",
  "ctx.closeOther": "Close Other Tabs",
  "ctx.closeRight": "Close Tabs to the Right",
  "ctx.closeTabGroup": "Close Tab Group",
  "ctx.closeAll": "Close All Tabs",
  "ctx.lockSession": "Lock Session...",
  "ctx.cloneSession": "Clone Session",
  "ctx.cloneNewWindow": "Clone in New Window",
  "ctx.sendToNewWindow": "Send to New Window",
  "ctx.sendToNewTabGroup": "Send to New Tab Group",
  "ctx.connectSftp": "Connect SFTP Session",
  "ctx.openSecureFX": "Open SecureFX",
  "ctx.sessionOptions": "Session Options...",
  "ctx.saveSession": "Save Session...",
  "ctx.sendCommandsToGroup": "Send Commands to This Group",

  // Performance panel
  "perf.title": "Performance",
  "perf.close": "Close performance panel",
  "perf.intervalTip": "Sample interval",
  "perf.intervalOpt": "{s}s",
  "perf.noTab": "No active tab: connect to a server first",
  "perf.connecting": "Connecting…",
  "perf.waiting": "Sampling, waiting for the second point",
  "perf.failed": "Sampling failed",
  "perf.noData": "No samples from this host: needs Linux /proc and sh",
  "perf.cpu": "CPU",
  "perf.mem": "Memory",
  "perf.net": "Network",
  "perf.disk": "Disk /",
  "perf.loadTip": "Load",
  "perf.memUsedShort": "used",
  "perf.netSub": "rx / tx",
  "perf.peak": "peak",
  "perf.diskFree": "free",
  "perf.up": "up",
  "perf.samples": "samples",
  "perf.footSource": "Read from the target's /proc and df",
};

const dicts: Record<Locale, Dict> = { zh, en };

export const LOCALES: Locale[] = ["zh", "en"];

/// 指定语言的文案，缺失时返回 undefined（不退回当前语言）：
/// 判断"这个名字是不是某语言的出厂默认名"要用它，见 App 里的面板页签改名。
export function tLocale(l: Locale, key: string): string | undefined {
  return dicts[l][key];
}

function detect(): Locale {
  const saved = localStorage.getItem(STORE_KEY);
  if (saved === "zh" || saved === "en") return saved;
  return navigator.language.toLowerCase().startsWith("zh") ? "zh" : "en";
}

let locale: Locale = detect();
const subs = new Set<() => void>();

const HTML_LANG: Record<Locale, string> = { zh: "zh-CN", en: "en" };
function applyLang(l: Locale): void {
  document.documentElement.lang = HTML_LANG[l];
}
applyLang(locale);

export function getLocale(): Locale {
  return locale;
}

export function setLocale(l: Locale): void {
  if (l === locale) return;
  locale = l;
  applyLang(l);
  try {
    localStorage.setItem(STORE_KEY, l);
  } catch {
    /* 忽略持久化失败 */
  }
  subs.forEach((f) => f());
}

function subscribe(f: () => void): () => void {
  subs.add(f);
  return () => subs.delete(f);
}

/// 取当前语言的文案；params 里的 {key} 会被替换。缺失的 key 退回英文再退回 key 本身。
export function t(key: string, params?: Record<string, string | number>): string {
  let s = dicts[locale][key] ?? dicts.en[key] ?? key;
  if (params) for (const k in params) s = s.split(`{${k}}`).join(String(params[k]));
  return s;
}

/// 组件里用它订阅语言变化：切换语言时触发重渲染，t() 随即读到新文案。
export function useI18n(): { t: typeof t; locale: Locale; setLocale: typeof setLocale } {
  useSyncExternalStore(subscribe, getLocale, getLocale);
  return { t, locale, setLocale };
}
