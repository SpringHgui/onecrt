use gpui_kit as gpui;

use gpui::SharedString;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Zh,
    En,
}

/// 中英成对给出，省掉一张键表：调用点就是唯一事实来源
pub fn tr(lang: Lang, zh: &'static str, en: &'static str) -> SharedString {
    if lang == Lang::Zh {
        zh.into()
    } else {
        en.into()
    }
}
