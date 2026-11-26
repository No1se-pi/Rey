use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle,
};
use std::{path::PathBuf, sync::Arc};

pub const BASE: Color32 = Color32::from_rgb(19, 17, 27);
pub const SIDE: Color32 = Color32::from_rgb(23, 20, 32);
pub const PANEL: Color32 = Color32::from_rgb(31, 26, 44);
pub const RAISED: Color32 = Color32::from_rgb(41, 34, 57);
pub const LINE: Color32 = Color32::from_rgb(54, 45, 71);
pub const TEXT: Color32 = Color32::from_rgb(236, 228, 246);
pub const MUTED: Color32 = Color32::from_rgb(166, 151, 187);
pub const ACCENT: Color32 = Color32::from_rgb(195, 163, 239);
pub const MINT: Color32 = Color32::from_rgb(147, 213, 184);
pub const ERROR: Color32 = Color32::from_rgb(244, 157, 175);

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(TEXT);
    visuals.panel_fill = BASE;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = SIDE;
    visuals.faint_bg_color = RAISED;
    visuals.window_stroke = Stroke::new(1.0_f32, LINE);
    visuals.selection.bg_fill = Color32::from_rgb(101, 71, 140);
    visuals.hyperlink_color = ACCENT;
    for (widget, fill, stroke) in [
        (&mut visuals.widgets.noninteractive, PANEL, MUTED),
        (&mut visuals.widgets.inactive, PANEL, TEXT),
        (&mut visuals.widgets.hovered, RAISED, ACCENT),
        (
            &mut visuals.widgets.active,
            Color32::from_rgb(91, 63, 125),
            TEXT,
        ),
        (&mut visuals.widgets.open, RAISED, TEXT),
    ] {
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.bg_stroke = Stroke::new(1.0_f32, LINE);
        widget.fg_stroke = Stroke::new(1.0_f32, stroke);
        widget.corner_radius = CornerRadius::same(9);
    }
    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.interact_size.y = 32.0;
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(24.0));
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(15.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(14.0));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(12.0));
    ctx.set_style(style);
    let mut fonts = FontDefinitions::default();
    for (name, path) in system_fonts() {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert(name.to_owned(), Arc::new(FontData::from_owned(bytes)));
            for family in [FontFamily::Proportional, FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .push(name.to_owned());
            }
        }
    }
    ctx.set_fonts(fonts);
}

fn system_fonts() -> Vec<(&'static str, PathBuf)> {
    #[cfg(target_os = "linux")]
    {
        let match_font = |pattern: &str| -> Option<PathBuf> {
            let result = std::process::Command::new("fc-match")
                .args(["-f", "%{file}", pattern])
                .output()
                .ok()?;
            result
                .status
                .success()
                .then(|| PathBuf::from(String::from_utf8_lossy(&result.stdout).as_ref()))
        };
        [
            ("cyrillic", "DejaVu Sans"),
            ("japanese", ":lang=ja"),
            ("symbols", "DejaVu Sans:charset=2654"),
        ]
        .into_iter()
        .filter_map(|(name, pattern)| match_font(pattern).map(|path| (name, path)))
        .collect()
    }
    #[cfg(target_os = "windows")]
    {
        let directory = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join("Fonts");
        [
            ("cyrillic", "segoeui.ttf"),
            ("japanese", "meiryo.ttc"),
            ("symbols", "seguisym.ttf"),
        ]
        .into_iter()
        .map(|(name, file)| (name, directory.join(file)))
        .collect()
    }
}

pub fn card(fill: Color32) -> egui::Frame {
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0_f32, LINE))
        .corner_radius(14)
        .inner_margin(egui::Margin::same(14))
}

pub fn primary(text: &str) -> egui::Button<'_> {
    egui::Button::new(
        egui::RichText::new(text)
            .strong()
            .color(Color32::from_rgb(28, 18, 40)),
    )
    .fill(ACCENT)
    .stroke(Stroke::NONE)
    .corner_radius(9)
}
