//! 配色与字体：界面 token 直接沿用旧版 styles.css 的取样值，终端调色板
//! 与 xterm 默认一致，只覆盖红/黄/绿三档（旧版也是这么做的）。

use gpui_kit as gpui;

use alacritty_terminal::vte::ansi::Rgb;
use gpui::{px, rgb, Font, FontFallbacks, Hsla, Pixels, Rgba};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Dark,
    Light,
}

#[derive(Clone)]
pub struct Theme {
    pub mode: Mode,

    pub bg: Hsla,
    pub bg_content: Hsla,
    pub bg_hover: Hsla,
    pub bg_selected: Hsla,
    pub bg_active: Hsla,
    pub bg_pill: Hsla,
    pub line: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub menu_bg: Hsla,
    pub field: Hsla,
    pub selection: Hsla,
    pub text: Hsla,
    pub text_2: Hsla,
    pub text_3: Hsla,
    pub accent: Hsla,
    pub accent_hover: Hsla,
    pub danger: Hsla,
    pub dot_warn: Hsla,
    pub dot_error: Hsla,

    pub term_bg: Hsla,
    pub term_fg: Hsla,
    pub term_cursor: Hsla,
    pub term_selection: Hsla,
    pub ansi: [Rgb; 16],

    pub ui_font: Font,
    pub mono_font: Font,
    pub ui_size: Pixels,
    pub mono_size: Pixels,
}

fn ansi_rgb(hex: u32) -> Rgb {
    Rgb {
        r: ((hex >> 16) & 0xff) as u8,
        g: ((hex >> 8) & 0xff) as u8,
        b: (hex & 0xff) as u8,
    }
}

fn h(hex: u32) -> Hsla {
    Rgba::from(rgb(hex)).into()
}

fn ui_font() -> Font {
    Font {
        family: "Microsoft YaHei".into(),
        fallbacks: Some(FontFallbacks::from_fonts(vec![
            "Segoe UI".into(),
            "PingFang SC".into(),
            "Noto Sans CJK SC".into(),
        ])),
        ..Default::default()
    }
}

fn mono_font() -> Font {
    Font {
        family: "Consolas".into(),
        fallbacks: Some(FontFallbacks::from_fonts(vec![
            "Cascadia Mono".into(),
            "Microsoft YaHei".into(),
            "PingFang SC".into(),
            "Noto Sans CJK SC".into(),
        ])),
        ..Default::default()
    }
}

impl Theme {
    pub fn new(mode: Mode, mono_size: f32) -> Self {
        if mode == Mode::Dark {
            Self {
                mode,
                bg: h(0x222426),
                bg_content: h(0x181a1b),
                bg_hover: h(0x2a2c2e),
                bg_selected: h(0x3e3f41),
                bg_active: h(0x46484a),
                bg_pill: h(0x2c2e2e),
                line: h(0x121517),
                border: h(0x303233),
                border_strong: h(0x3a3c3e),
                menu_bg: h(0x252627),
                field: h(0x2c2e2e),
                selection: h(0x3e3f41),
                text: h(0xe7e9ed),
                text_2: h(0xa2a5a7),
                text_3: h(0x75787a),
                accent: h(0x27bd51),
                accent_hover: h(0x3fd066),
                danger: h(0xff9c99),
                dot_warn: h(0xd8a93c),
                dot_error: h(0xf0605d),
                term_bg: h(0x181a1b),
                term_fg: h(0xe7e9ed),
                term_cursor: h(0x27bd51),
                term_selection: h(0x2e8b57),
                ansi: ansi_palette(true),
                ui_font: ui_font(),
                mono_font: mono_font(),
                ui_size: px(13.),
                mono_size: px(mono_size),
            }
        } else {
            Self {
                mode,
                bg: h(0xf6f7fa),
                bg_content: h(0xffffff),
                bg_hover: h(0xeef1f6),
                bg_selected: h(0xe4e8ef),
                bg_active: h(0xdde3ec),
                bg_pill: h(0xf0f0f2),
                line: h(0xe9eaec),
                border: h(0xdddddd),
                border_strong: h(0xcdd2dc),
                menu_bg: h(0xf8f8f8),
                field: h(0xf0f0f2),
                selection: h(0xdfe7f2),
                text: h(0x2b3547),
                text_2: h(0x727985),
                text_3: h(0x9fa4ac),
                accent: h(0x35cc5f),
                accent_hover: h(0x2dbb54),
                danger: h(0xb3352e),
                dot_warn: h(0xd8a93c),
                dot_error: h(0xf0605d),
                term_bg: h(0xffffff),
                term_fg: h(0x2b3547),
                term_cursor: h(0x35cc5f),
                term_selection: h(0xa6d8b8),
                ansi: ansi_palette(false),
                ui_font: ui_font(),
                mono_font: mono_font(),
                ui_size: px(13.),
                mono_size: px(mono_size),
            }
        }
    }

    pub fn toggle(&self, mono_size: f32) -> Self {
        Self::new(
            match self.mode {
                Mode::Dark => Mode::Light,
                Mode::Light => Mode::Dark,
            },
            mono_size,
        )
    }
}

/// 把自绘 UI 的明暗与字体同步给 gpui-component，输入框/菜单/对话框才不会跳色。
/// 组件自带主题的灰阶和旧版差得远，这里逐项覆盖成旧版 styles.css 的取样值。
pub fn sync_component(mode: Mode, cx: &mut gpui::App) {
    use gpui_kit::component::{Theme as UiTheme, ThemeMode};
    UiTheme::change(
        if mode == Mode::Dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        None,
        cx,
    );
    let ours = Theme::new(mode, 14.);
    let white = h(0xffffff);
    UiTheme::update(cx, |theme| {
        theme.font_family = "Microsoft YaHei".into();
        theme.mono_font_family = "Consolas".into();
        theme.font_size = px(13.);
        theme.mono_font_size = px(13.);
        theme.radius = px(6.);

        let c = &mut theme.colors;
        c.background = ours.bg;
        c.foreground = ours.text;
        c.border = ours.border;
        c.input = ours.border_strong;
        c.muted = ours.bg_pill;
        c.muted_foreground = ours.text_3;
        c.overlay = if mode == Mode::Dark {
            h(0x000000).opacity(0.6)
        } else {
            h(0x2b3547).opacity(0.28)
        };

        c.accent = ours.bg_selected;
        c.accent_foreground = ours.text;
        c.selection = ours.selection;
        c.caret = ours.accent;
        c.ring = ours.accent.opacity(0.35);

        c.popover = ours.menu_bg;
        c.popover_foreground = ours.text;

        c.primary = ours.accent;
        c.primary_hover = ours.accent_hover;
        c.primary_active = ours.accent_hover;
        c.primary_foreground = white;
        c.button_primary = ours.accent;
        c.button_primary_hover = ours.accent_hover;
        c.button_primary_active = ours.accent_hover;
        c.button_primary_foreground = white;

        c.secondary = ours.bg_pill;
        c.secondary_hover = ours.bg_hover;
        c.secondary_active = ours.bg_active;
        c.secondary_foreground = ours.text;
        c.button = ours.field;
        c.button_hover = ours.bg_hover;
        c.button_active = ours.bg_active;
        c.button_foreground = ours.text;
        c.button_secondary = ours.field;
        c.button_secondary_hover = ours.bg_hover;
        c.button_secondary_active = ours.bg_active;
        c.button_secondary_foreground = ours.text;

        c.danger = ours.dot_error;
        c.danger_hover = h(0xf37a77);
        c.danger_active = h(0xd94b48);
        c.danger_foreground = white;
        c.button_danger = ours.dot_error;
        c.button_danger_hover = h(0xf37a77);
        c.button_danger_active = h(0xd94b48);
        c.button_danger_foreground = white;

        c.link = ours.accent;
        c.link_hover = ours.accent_hover;
        c.link_active = ours.accent_hover;

        c.sidebar = ours.bg;
        c.sidebar_border = ours.border_strong;
        c.sidebar_foreground = ours.text;
        c.sidebar_accent = ours.bg_active;
        c.sidebar_accent_foreground = ours.text;
        c.sidebar_primary = ours.accent;
        c.sidebar_primary_foreground = white;

        c.tab = ours.bg;
        c.tab_active = ours.bg_content;
        c.tab_bar = ours.bg;
        c.tab_foreground = ours.text_2;
        c.tab_active_foreground = ours.text;
        c.tab_bar_segmented = ours.bg_pill;

        c.list = ours.bg_content;
        c.list_hover = ours.bg_hover;
        c.list_active = ours.bg_selected;
        c.list_active_border = ours.border_strong;
        c.list_head = ours.bg;
        c.list_even = ours.bg;

        c.title_bar = ours.bg;
        c.title_bar_border = ours.border;
        c.status_bar = ours.bg;
        c.status_bar_border = ours.border;

        c.scrollbar = h(0x000000).opacity(0.0);
        c.scrollbar_thumb = ours.border_strong.opacity(0.9);
        c.scrollbar_thumb_hover = ours.text_3.opacity(0.8);
        c.drag_border = ours.border_strong;
        c.drop_target = ours.selection;
        c.switch = ours.border_strong;
        c.switch_thumb = white;
        c.skeleton = ours.bg_pill;
        c.slider_bar = ours.bg_active;
        c.slider_thumb = ours.accent;
        c.progress_bar = ours.bg_active;
        c.window_border = ours.border_strong;
        c.group_box = ours.bg_content;
        c.group_box_foreground = ours.text;
        c.accordion = ours.bg_content;
        c.description_list_label = ours.bg_pill;
        c.description_list_label_foreground = ours.text_2;
        c.table = ours.bg_content;
        c.table_even = ours.bg;
        c.table_head = ours.bg;
        c.table_head_foreground = ours.text_2;
        c.table_foot = ours.bg;
        c.table_foot_foreground = ours.text_2;
        c.table_hover = ours.bg_hover;
        c.table_active = ours.bg_selected;
        c.table_active_border = ours.border_strong;
        c.table_row_border = ours.border;

        c.red = h(0xf0605d);
        c.red_light = h(0xff9c99);
        c.green = ours.accent;
        c.green_light = ours.accent_hover;
        c.blue = h(0x3478f6);
        c.blue_light = h(0x6b9ff8);
        c.yellow = ours.dot_warn;
        c.yellow_light = h(0xe8c56a);
        c.magenta = h(0xc650c6);
        c.magenta_light = h(0xdc7adc);
        c.cyan = h(0x2aa3a3);
        c.cyan_light = h(0x5bc0c0);
    });
}

fn ansi_palette(dark: bool) -> [Rgb; 16] {
    let (red, yellow, green) = if dark {
        (0xf14c4c, 0xe2bb6f, 0x27bd51)
    } else {
        (0xb3352e, 0x8a6a12, 0x177038)
    };
    let base = [
        0x000000, red, green, yellow, 0x0000ee, 0xcd00cd, 0x00cdcd, 0xe5e5e5, 0x7f7f7f, red, green,
        yellow, 0x5c5cff, 0xff00ff, 0x00ffff, 0xffffff,
    ];
    let mut out = [ansi_rgb(base[0]); 16];
    for (slot, hex) in out.iter_mut().zip(base) {
        *slot = ansi_rgb(hex);
    }
    out
}

/// 终端里 24 位色也要经过背景/前景这套 token，否则亮主题下白字看不见
pub fn term_rgb(rgb: Rgb) -> Hsla {
    Rgba {
        r: rgb.r as f32 / 255.0,
        g: rgb.g as f32 / 255.0,
        b: rgb.b as f32 / 255.0,
        a: 1.0,
    }
    .into()
}
