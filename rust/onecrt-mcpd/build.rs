fn main() {
    // 任务管理器的「启动应用」和资源管理器都从 exe 资源里取图标，
    // 后台服务不嵌图标就显示成通用空白图标。
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/icon.rc");
        println!("cargo:rerun-if-changed=../onecrt/assets/onecrt.ico");
        embed_resource::compile("assets/icon.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("嵌入应用图标失败");
    }
}
