// Windows 默认把 [[bin]] 链接成控制台子系统（PE Subsystem=3 CUI），从资源管理器双击、
// 桌面快捷方式或安装完的「运行 OneCRT」启动时，系统会先给进程新建一个控制台，
// 于是主窗口前面多出一个黑色命令行窗口。这里在非 debug 构建切成 GUI 子系统（=2）。
// debug 故意保留控制台：panic（例如主线程栈溢出）和 eprintln 只有在那个窗口里看得见，
// 否则调试期崩掉就是彻底无声的「点了没反应」。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod about_dialog;
mod app;
mod command_form;
mod dialog_window;
mod file_panel;
mod i18n;
mod import_flow;
mod mcp_dialog;
mod move_paint_skip;
mod password_prompt;
mod perf_panel;
mod quick_connect;
mod session_form;
mod term_engine;
mod term_view;
mod theme;
mod update;
mod window_drag;

use std::sync::Arc;

use gpui_kit as gpui;
use gpui_kit::component::Root;
use gpui_kit::AppContext;

fn main() {
    gpui::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(|cx: &mut gpui::App| {
            // gpui-kit 的 init 同时初始化 gpui-component（输入框、菜单、对话框都靠它）
            gpui_kit::init(cx);
            // Root 把 tab / shift-tab 绑成了窗口内的焦点跳转，且绑定先于按键监听执行，
            // 终端自己的 stop_propagation 拦不住。这里在终端上下文里用 NoAction 压掉它，
            // 让 Tab 照常落到终端、送进 PTY。
            cx.bind_keys([
                gpui::KeyBinding::new("tab", gpui::NoAction {}, Some("Terminal")),
                gpui::KeyBinding::new("shift-tab", gpui::NoAction {}, Some("Terminal")),
            ]);

            let backend = Arc::new(onecrt_core::Backend::new().expect("初始化后台运行时失败"));
            onecrt_core::boot_mcp(backend.clone(), || {});
            let options = app::window_options(cx);
            gpui::open_window(options, cx, move |window, cx| {
                let backend = backend.clone();
                // 拖动窗口时跳过移动循环里的整帧重绘，否则大窗口一帧几十毫秒，
                // 会把模态移动循环堵成 ~20Hz 的跳格。
                move_paint_skip::install(window);
                let view = cx.new(|cx| app::OneCrt::new(backend, window, cx));
                let root = cx.new(|cx| Root::new(view, window, cx));
                root
            })
            .expect("打开主窗口失败");
            cx.activate(true);
        });
}
