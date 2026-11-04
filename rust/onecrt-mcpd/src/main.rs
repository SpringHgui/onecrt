//! OneCRT 的 MCP 后台服务：只跑 `mcp.rs` 里的监听，不带任何界面。
//!
//! 意图全部来自 shellx 配置目录里的 `mcp.json`（由主程序写），这个进程每秒跟一次；
//! 它把自己在不在监听写进同目录的 `mcpd.json`，主程序只读心跳。
//! 登录自启的登记由主程序负责，所以调试期手动跑它不会被写进系统启动项。

// 后台服务不该有控制台窗口；调试构建保留，panic 才看得见。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("onecrt-mcpd")
        .build()
        .expect("初始化 MCP 后台运行时失败");
    if let Err(e) = rt.block_on(onecrt_core::mcp::run(onecrt_core::terminal::new_manager())) {
        eprintln!("onecrt-mcpd 退出: {e}");
        std::process::exit(1);
    }
}
