//! OneCRT 的无界面内核：会话存取、SSH 终端、SFTP、导入器、性能采集与 MCP 服务。
//!
//! 这里不依赖任何 GUI 框架；界面层通过 [`Backend`] 持有一份 tokio 运行时与几个
//! `Arc<Mutex<…>>` 管理器，把结果经回调/通道送回自己的事件循环。

pub mod files;
pub mod import;
pub mod mcp;
pub mod sessions;
pub mod settings;
pub mod ssh;
pub mod stats;
pub mod terminal;

use std::io;
use std::sync::Arc;

/// 界面层唯一需要持有的东西：一个 tokio 运行时 + 各管理器。
pub struct Backend {
    rt: tokio::runtime::Runtime,
    pub terminals: terminal::Shared,
    pub files: files::Shared,
    pub stats: stats::Shared,
    pub mcp: mcp::Shared,
}

impl Backend {
    pub fn new() -> io::Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("onecrt-io")
            .build()?;
        let terminals = terminal::new_manager();
        let files = files::new_manager();
        let stats = stats::new_manager();
        let mcp = mcp::new_manager(terminals.clone());
        Ok(Backend {
            rt,
            terminals,
            files,
            stats,
            mcp,
        })
    }

    /// 把一段异步任务丢到后台运行时。返回的 JoinHandle 在 tokio 运行时里，
    /// 界面层一般只关心通道里回来的结果，不持有它。
    pub fn spawn<F>(&self, fut: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.rt.spawn(fut)
    }

    pub fn handle(&self) -> tokio::runtime::Handle {
        self.rt.handle().clone()
    }
}

/// MCP 服务按上次落盘的开关在启动时自恢复
pub fn boot_mcp(backend: Arc<Backend>, notify: impl Fn() + Send + Sync + 'static) {
    let mcp = backend.mcp.clone();
    backend.spawn(async move {
        mcp::boot(&mcp).await;
        notify();
    });
}
