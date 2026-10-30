fn main() {
    // 图标资源只有 Windows 需要；其他平台不引入 embed-resource。
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/icon.rc");
        println!("cargo:rerun-if-changed=assets/onecrt.ico");
        embed_resource::compile("assets/icon.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("嵌入应用图标失败");
    }

    // GPUI 布局是自上而下递归 request_layout/paint，元素树每深一层就多一整套
    // （AnyElement → Div → Interactivity → with_text_style/with_image_cache）栈帧，
    // debug 构建实测每层约 20-25KB。主窗口的停靠结构本身已经吃掉 ~700KB，再开文件面板
    // （工具条 + 双栏 + 目录列表）就顶穿 MSVC 默认 1MB（1048576 字节）主线程栈，
    // 报 "thread 'main' has overflowed its stack" 直接退进程；从资源管理器双击启动时
    // 控制台一闪而过，用户只看到「打开文件面板就闪退」。抬到 16MB：只是预留虚拟地址，
    // 页按需提交，不占实际内存。macOS/Linux 主线程默认 8MB，无需处理。
    #[cfg(all(windows, target_env = "msvc"))]
    {
        println!("cargo:rustc-link-arg=/STACK:16777216");
    }
}
