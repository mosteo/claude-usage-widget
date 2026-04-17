use std::time::{Duration, Instant, SystemTime};

use eframe::egui;

use crate::config::Config;
use crate::cookies::BrowserKind;
use crate::usage_state::UsageModel;

const BG: egui::Color32 = egui::Color32::from_rgb(0xf5, 0xf5, 0xf0);
const FG: egui::Color32 = egui::Color32::from_rgb(0x1a, 0x1a, 0x1a);
const DIM: egui::Color32 = egui::Color32::from_rgb(0x88, 0x88, 0x88);
const BAR_BG: egui::Color32 = egui::Color32::from_rgb(0xd9, 0xd9, 0xd9);
const BAR_BLUE: egui::Color32 = egui::Color32::from_rgb(0x4a, 0x90, 0xd9);
const BAR_YELLOW: egui::Color32 = egui::Color32::from_rgb(0xd4, 0xa0, 0x17);
const BAR_RED: egui::Color32 = egui::Color32::from_rgb(0xdc, 0x45, 0x45);
const FOOTER_DIM: egui::Color32 = egui::Color32::from_rgb(0xaa, 0xaa, 0xaa);

const BAR_W: f32 = 124.0;
const BAR_H: f32 = 10.0;
const TICK: Duration = Duration::from_secs(30);
const PADDING: f32 = 10.0;
const MIN_HEIGHT: f32 = 274.0;
const SNAP_SECS: f64 = 0.7;
const SNAP_STAGGER: f64 = 0.12;

const REFRESH_OPTIONS: &[(u64, &str)] = &[
    (60, "1 min"),
    (120, "2 min"),
    (300, "5 min"),
    (600, "10 min"),
    (900, "15 min"),
    (1800, "30 min"),
];

const WEEKLY_KEYS: &[(&str, &str)] = &[
    ("seven_day", "All models"),
    ("seven_day_opus", "Opus"),
    ("seven_day_sonnet", "Sonnet"),
    ("seven_day_cowork", "Cowork"),
];

fn ease_out_back(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    let c1: f64 = 2.5;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

fn with_alpha(c: egui::Color32, a: f32) -> egui::Color32 {
    let [r, g, b, _] = c.to_array();
    egui::Color32::from_rgba_unmultiplied(r, g, b, (a.clamp(0.0, 1.0) * 255.0) as u8)
}

fn bar_color(pct: f64) -> egui::Color32 {
    if pct < 75.0 {
        BAR_BLUE
    } else if pct < 90.0 {
        BAR_YELLOW
    } else {
        BAR_RED
    }
}

fn time_left(resets_at: Option<&str>) -> String {
    let Some(s) = resets_at else {
        return String::new();
    };
    let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) else {
        return String::new();
    };
    let now = chrono::Utc::now();
    let secs = (dt.with_timezone(&chrono::Utc) - now).num_seconds();
    if secs <= 0 {
        return String::from("Resetting...");
    }
    let m = (secs / 60) % 60;
    let h = secs / 3600;
    if h >= 24 {
        format!("Resets in {}d {}h", h / 24, h % 24)
    } else if h > 0 {
        format!("Resets in {h} hr {m} min")
    } else {
        format!("Resets in {m} min")
    }
}

fn updated_ago(last: SystemTime) -> String {
    let Ok(ago) = SystemTime::now().duration_since(last) else {
        return String::from("Last updated: just now");
    };
    let secs = ago.as_secs();
    if secs < 60 {
        String::from("Last updated: just now")
    } else if secs < 3600 {
        let m = secs / 60;
        let s = if m != 1 { "s" } else { "" };
        format!("Last updated: {m} minute{s} ago")
    } else {
        let h = secs / 3600;
        let s = if h != 1 { "s" } else { "" };
        format!("Last updated: {h} hour{s} ago")
    }
}

pub struct UsageApp {
    model: UsageModel,
    title: String,
    title_explicit: bool,
    first_frame: bool,
    last_height: f32,
    data_arrived_at: Option<Instant>,
    had_focus: bool,
}

impl UsageApp {
    pub fn new(
        browser: BrowserKind,
        data_dir: Option<String>,
        oauth_dir: Option<String>,
        title: String,
        title_explicit: bool,
        config: Config,
        initial_cookies: Option<crate::cookies::CookieJar>,
    ) -> Self {
        let data_arrived_at = config
            .last_usage_snapshot
            .as_ref()
            .map(|_| Instant::now() - Duration::from_secs(10));

        Self {
            model: UsageModel::new(browser, data_dir, oauth_dir, &config, initial_cookies),
            title,
            title_explicit,
            first_frame: true,
            last_height: 0.0,
            data_arrived_at,
            had_focus: false,
        }
    }
}

impl eframe::App for UsageApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.first_frame {
            self.first_frame = false;
            ctx.style_mut(|s| {
                s.interaction.selectable_labels = false;
                s.visuals.panel_fill = BG;
                let menu_bg = egui::Color32::from_rgb(0x2a, 0x2a, 0x2a);
                s.visuals.widgets.noninteractive.bg_fill = menu_bg;
                s.visuals.window_fill = menu_bg;
                s.visuals.window_stroke =
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(0x50, 0x50, 0x50));
            });
        }

        let focused = ctx.input(|i| i.focused);
        if focused {
            self.had_focus = true;
        } else if self.had_focus {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        let outcome = self.model.poll_result();
        if let Some(name) = outcome.account_name {
            if !self.title_explicit {
                self.title = name;
            }
        }
        if outcome.received_first_success {
            self.data_arrived_at = Some(Instant::now());
        }

        if self.model.should_refresh() {
            self.model
                .start_fetch(!self.title_explicit && self.title == "Plan Usage");
        }

        ctx.request_repaint_after(TICK);

        egui::CentralPanel::default().show(ctx, |ui| {
            let panel_rect = ui.max_rect();
            let bg_response = ui.interact(panel_rect, ui.id().with("drag"), egui::Sense::drag());

            let close_size = 22.0;
            let close_rect = egui::Rect::from_min_size(
                egui::pos2(
                    panel_rect.right() - close_size - 2.0,
                    panel_rect.top() + 2.0,
                ),
                egui::vec2(close_size, close_size),
            );
            let close_resp = ui.interact(close_rect, ui.id().with("close"), egui::Sense::click());
            let close_color = if close_resp.hovered() { FG } else { DIM };
            ui.painter().text(
                close_rect.center(),
                egui::Align2::CENTER_CENTER,
                "\u{00d7}",
                egui::FontId::proportional(16.0),
                close_color,
            );
            if close_resp.clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }

            if bg_response.drag_started() {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }

            bg_response.context_menu(|ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new("Refresh interval")
                            .strong()
                            .size(11.0)
                            .color(egui::Color32::WHITE),
                    );
                    ui.separator();
                    for &(secs, label) in REFRESH_OPTIONS {
                        let current = self.model.refresh_secs == secs;
                        let text = if current {
                            egui::RichText::new(label)
                                .size(13.0)
                                .color(egui::Color32::WHITE)
                                .font(egui::FontId::new(
                                    13.0,
                                    egui::FontFamily::Name("bold".into()),
                                ))
                        } else {
                            egui::RichText::new(label)
                                .size(11.0)
                                .color(egui::Color32::WHITE)
                        };
                        let resp = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
                        if resp.clicked() {
                            self.model.set_refresh_secs(secs);
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                    let link = ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.label(
                            egui::RichText::new("by: ")
                                .size(11.0)
                                .color(egui::Color32::WHITE),
                        );
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new("SmartAppsCo")
                                    .size(11.0)
                                    .color(egui::Color32::from_rgb(0x6a, 0xb0, 0xf0)),
                            )
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                    });
                    if link.inner.clicked() {
                        let _ = open::that("https://smartapps.co/from/claude-usage-widget");
                        ui.close_menu();
                    }
                });
            });

            let inner_rect = egui::Rect::from_min_max(
                panel_rect.min + egui::vec2(PADDING - 5.0, 0.0),
                panel_rect.max - egui::vec2(PADDING, PADDING),
            );

            let content = ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner_rect), |ui| {
                if !self.title.is_empty() {
                    ui.label(egui::RichText::new(&self.title).color(FG).size(16.0).font(
                        egui::FontId::new(16.0, egui::FontFamily::Name("bold".into())),
                    ));
                    ui.add_space(4.0);
                }

                match &self.model.cached_data {
                    None => Self::render_loading(ui, ctx),
                    Some(Err(err)) => {
                        ui.label(egui::RichText::new(err.as_str()).color(BAR_RED).size(12.0));
                    }
                    Some(Ok(data)) => {
                        let elapsed = self
                            .data_arrived_at
                            .map_or(f64::MAX, |t| t.elapsed().as_secs_f64());
                        let mut section_idx = 0usize;

                        if let Some(bucket) = data.get("five_hour") {
                            let t = ((elapsed - section_idx as f64 * SNAP_STAGGER) / SNAP_SECS)
                                .clamp(0.0, 1.0);
                            Self::render_section(
                                ui,
                                "Current session",
                                bucket.utilization.unwrap_or(0.0),
                                bucket.resets_at.as_deref(),
                                t,
                            );
                            section_idx += 1;
                        }

                        let weekly: Vec<_> = WEEKLY_KEYS
                            .iter()
                            .filter_map(|(k, label)| data.get(*k).map(|bucket| (*label, bucket)))
                            .collect();
                        if !weekly.is_empty() {
                            let header_t = ((elapsed - section_idx as f64 * SNAP_STAGGER)
                                / SNAP_SECS)
                                .clamp(0.0, 1.0);
                            let header_alpha = (header_t * 6.0).min(1.0) as f32;
                            ui.add_space(2.0);
                            ui.label(
                                egui::RichText::new("Weekly limits")
                                    .color(with_alpha(FG, header_alpha))
                                    .size(16.0)
                                    .font(egui::FontId::new(
                                        16.0,
                                        egui::FontFamily::Name("bold".into()),
                                    )),
                            );
                            ui.add_space(2.0);
                            for (label, bucket) in &weekly {
                                let t = ((elapsed - section_idx as f64 * SNAP_STAGGER) / SNAP_SECS)
                                    .clamp(0.0, 1.0);
                                Self::render_section(
                                    ui,
                                    label,
                                    bucket.utilization.unwrap_or(0.0),
                                    bucket.resets_at.as_deref(),
                                    t,
                                );
                                section_idx += 1;
                            }
                        }

                        let anim_end = section_idx as f64 * SNAP_STAGGER + SNAP_SECS;
                        if elapsed < anim_end {
                            ui.ctx().request_repaint_after(Duration::from_millis(16));
                        }
                    }
                }

                if let Some(last_fetch) = self.model.last_fetch {
                    let footer_alpha = self.data_arrived_at.map_or(1.0_f32, |arrived| {
                        let e = arrived.elapsed().as_secs_f64();
                        ((e - 0.3) * 4.0).clamp(0.0, 1.0) as f32
                    });
                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new(updated_ago(last_fetch))
                            .color(with_alpha(FOOTER_DIM, footer_alpha))
                            .size(10.0),
                    );
                }
            });

            let used_h = (content.response.rect.height() + PADDING * 2.0).max(MIN_HEIGHT);
            if (used_h - self.last_height).abs() > 0.5 {
                self.last_height = used_h;
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(186.0, used_h)));
            }
        });
    }
}

impl UsageApp {
    fn render_loading(ui: &mut egui::Ui, ctx: &egui::Context) {
        const PHRASES: &[&str] = &[
            "Warming up...",
            "Opening cookie jar...",
            "Counting tokens...",
            "Harmonizing...",
            "Consulting the oracle...",
            "Crunching numbers...",
            "Reticulating splines...",
            "Almost there...",
        ];

        let time = ctx.input(|i| i.time);
        let colors = [BAR_BLUE, BAR_YELLOW, BAR_RED];

        let bar_block_h = colors.len() as f32 * (BAR_H + 12.0) + 24.0;
        let available = ui.available_height();
        ui.add_space(((available - bar_block_h) / 2.0).max(0.0));

        for (i, &color) in colors.iter().enumerate() {
            ui.add_space(6.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(BAR_W, BAR_H), egui::Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 2.0, BAR_BG);

            let phase = time / 2.0 - i as f64 * 0.2;
            let t = (phase.fract() + 1.0).fract();
            let fill = (t * std::f64::consts::PI).sin() as f32;

            if fill > 0.01 {
                let fill_rect =
                    egui::Rect::from_min_size(rect.min, egui::vec2(BAR_W * fill, BAR_H));
                painter.rect_filled(fill_rect, 2.0, color);
            }
            ui.add_space(6.0);
        }

        let idx = (time / 2.5) as usize % PHRASES.len();
        ui.add_space(8.0);
        ui.label(egui::RichText::new(PHRASES[idx]).color(DIM).size(11.0));

        ctx.request_repaint_after(Duration::from_millis(30));
    }

    fn render_section(
        ui: &mut egui::Ui,
        label: &str,
        utilization: f64,
        resets_at: Option<&str>,
        anim_t: f64,
    ) {
        let bar_t = ease_out_back(anim_t);
        let text_alpha = (anim_t * 6.0).min(1.0) as f32;

        let pct = utilization.round().min(100.0);
        let color = bar_color(pct);

        ui.label(
            egui::RichText::new(label)
                .color(with_alpha(FG, text_alpha))
                .font(egui::FontId::new(
                    13.0,
                    egui::FontFamily::Name("bold".into()),
                )),
        );
        let reset_text = time_left(resets_at);
        if !reset_text.is_empty() {
            ui.label(
                egui::RichText::new(&reset_text)
                    .color(with_alpha(DIM, text_alpha))
                    .size(11.0),
            );
        }

        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(BAR_W, BAR_H), egui::Sense::hover());
            let painter = ui.painter_at(rect);

            let h_scale = bar_t.clamp(0.0, 1.15) as f32;
            let draw_h = BAR_H * h_scale;
            let y_off = (BAR_H - draw_h) / 2.0;
            let bar_rect = egui::Rect::from_min_size(
                rect.min + egui::vec2(0.0, y_off),
                egui::vec2(BAR_W, draw_h),
            );
            painter.rect_filled(bar_rect, 2.0, BAR_BG);

            let fill_w = (BAR_W * (pct as f32) / 100.0 * bar_t as f32).min(BAR_W);
            if fill_w > 0.0 {
                let fill_rect = egui::Rect::from_min_size(bar_rect.min, egui::vec2(fill_w, draw_h));
                painter.rect_filled(fill_rect, 2.0, color);
            }

            ui.add_space(3.0);
            let display_pct = (pct * bar_t).min(100.0);
            ui.label(
                egui::RichText::new(format!("{display_pct:.0}%"))
                    .color(with_alpha(DIM, text_alpha))
                    .size(11.0),
            );
        });
        ui.add_space(4.0);
    }
}
