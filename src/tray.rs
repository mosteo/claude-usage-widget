#![cfg(target_os = "linux")]

use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::api::UsageResponse;
use crate::config;
use crate::cookies::{self, BrowserKind};
use crate::usage_state::UsageModel;

const POPUP_WIDTH: i32 = 186;
const POPUP_HEIGHT: i32 = 274;
const POPUP_MARGIN: i32 = 12;
const TRAY_POLL_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Clone)]
pub struct TrayOptions {
    browser: Option<BrowserKind>,
    data_dir: Option<String>,
    oauth_dir: Option<String>,
    title: Option<String>,
}

impl TrayOptions {
    pub fn from_cli(options: &crate::CliOptions) -> Self {
        Self {
            browser: options.browser,
            data_dir: options.data_dir.clone(),
            oauth_dir: options.oauth_dir.clone(),
            title: options.title_explicit.then_some(options.title.clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct TrayMetrics {
    five_hour: String,
    five_hour_reset: Option<f64>,
    weekly: String,
    weekly_reset: Option<f64>,
    title: String,
    tooltip: String,
}

impl Default for TrayMetrics {
    fn default() -> Self {
        Self {
            five_hour: String::from("%?"),
            five_hour_reset: None,
            weekly: String::from("%?"),
            weekly_reset: None,
            title: String::from("5h %? | 7d %?"),
            tooltip: String::from("Claude Usage\n5h: %?\n7d: %?"),
        }
    }
}

impl TrayMetrics {
    fn from_usage(data: &UsageResponse) -> Self {
        let five_hour_bucket = data.get("five_hour");
        let weekly_bucket = data
            .get("seven_day")
            .or_else(|| data.get("seven_day_opus"))
            .or_else(|| data.get("seven_day_sonnet"))
            .or_else(|| data.get("seven_day_cowork"));

        let five_hour = percent_string(five_hour_bucket.and_then(|bucket| bucket.utilization));
        let weekly = percent_string(weekly_bucket.and_then(|bucket| bucket.utilization));

        Self {
            five_hour: five_hour.clone(),
            five_hour_reset: reset_fraction(
                five_hour_bucket.and_then(|bucket| bucket.resets_at.as_deref()),
                5 * 60 * 60,
            ),
            weekly: weekly.clone(),
            weekly_reset: reset_fraction(
                weekly_bucket.and_then(|bucket| bucket.resets_at.as_deref()),
                7 * 24 * 60 * 60,
            ),
            title: format!("5h {five_hour} | 7d {weekly}"),
            tooltip: format!("Claude Usage\n5h: {five_hour}\n7d: {weekly}"),
        }
    }
}

fn fill_rect(buf: &mut [u8], width: usize, x: usize, y: usize, w: usize, h: usize, color: [u8; 4]) {
    for yy in y..(y + h) {
        for xx in x..(x + w) {
            let idx = (yy * width + xx) * 4;
            buf[idx..idx + 4].copy_from_slice(&color);
        }
    }
}

fn parse_percent(text: &str) -> Option<f64> {
    text.strip_suffix('%')?.parse::<f64>().ok()
}

fn palette_color(percent: Option<f64>) -> [u8; 4] {
    match percent {
        None => [150, 150, 150, 255],
        Some(p) if p < 20.0 => [74, 144, 217, 255],
        Some(p) if p < 40.0 => [58, 166, 85, 255],
        Some(p) if p < 60.0 => [212, 190, 57, 255],
        Some(p) if p < 80.0 => [224, 138, 47, 255],
        Some(_) => [220, 69, 69, 255],
    }
}

fn reset_bar_color() -> [u8; 4] {
    [198, 154, 255, 255]
}

fn reset_fraction(resets_at: Option<&str>, total_secs: i64) -> Option<f64> {
    let resets_at = resets_at?;
    let reset_dt = chrono::DateTime::parse_from_rfc3339(resets_at).ok()?;
    let remaining = (reset_dt.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_seconds();
    Some((remaining as f64 / total_secs as f64).clamp(0.0, 1.0))
}

fn draw_bar(
    buf: &mut [u8],
    width: usize,
    x: usize,
    y: usize,
    bar_w: usize,
    bar_h: usize,
    percent: Option<f64>,
) {
    let frame = [190, 190, 190, 255];
    let bg = [255, 255, 255, 0];
    fill_rect(buf, width, x, y, bar_w, bar_h, frame);
    fill_rect(buf, width, x + 1, y + 1, bar_w - 2, bar_h - 2, bg);

    let fill_pct = percent.unwrap_or(0.0).clamp(0.0, 100.0);
    let inner_h = bar_h - 2;
    let filled_h = ((inner_h as f64) * fill_pct / 100.0).round() as usize;
    if filled_h > 0 {
        let fill_y = y + 1 + inner_h - filled_h;
        fill_rect(
            buf,
            width,
            x + 1,
            fill_y,
            bar_w - 2,
            filled_h,
            palette_color(percent),
        );
    }
}

fn draw_reset_bar(
    buf: &mut [u8],
    width: usize,
    x: usize,
    y: usize,
    bar_w: usize,
    bar_h: usize,
    fraction_remaining: Option<f64>,
) {
    let frame = [195, 180, 220, 210];
    let bg = [255, 255, 255, 0];
    fill_rect(buf, width, x, y, bar_w, bar_h, frame);
    fill_rect(buf, width, x + 1, y + 1, bar_w - 2, bar_h - 2, bg);

    let fill = fraction_remaining.unwrap_or(0.0).clamp(0.0, 1.0);
    let inner_h = bar_h - 2;
    let filled_h = ((inner_h as f64) * fill).round() as usize;
    if filled_h > 0 {
        let fill_y = y + 1 + inner_h - filled_h;
        fill_rect(
            buf,
            width,
            x + 1,
            fill_y,
            bar_w - 2,
            filled_h,
            reset_bar_color(),
        );
    }
}

fn rgba_to_argb(rgba: &[u8]) -> Vec<u8> {
    let mut argb = Vec::with_capacity(rgba.len());
    for chunk in rgba.chunks_exact(4) {
        argb.extend_from_slice(&[chunk[3], chunk[0], chunk[1], chunk[2]]);
    }
    argb
}

fn tray_icon_pixmap(metrics: &TrayMetrics) -> Vec<ksni::Icon> {
    const SIZE: usize = 48;
    const MAIN_BAR_W: usize = 12;
    const RESET_BAR_W: usize = 5;
    const GROUP_W: usize = MAIN_BAR_W + 1 + RESET_BAR_W;
    const BAR_H: usize = 34;
    const BAR_Y: usize = 7;
    const LEFT_X: usize = 7;
    const RIGHT_X: usize = 26;
    let mut rgba = vec![0u8; SIZE * SIZE * 4];

    draw_bar(
        &mut rgba,
        SIZE,
        LEFT_X,
        BAR_Y,
        MAIN_BAR_W,
        BAR_H,
        parse_percent(&metrics.five_hour),
    );
    draw_reset_bar(
        &mut rgba,
        SIZE,
        LEFT_X + GROUP_W - RESET_BAR_W,
        BAR_Y,
        RESET_BAR_W,
        BAR_H,
        metrics.five_hour_reset,
    );
    draw_bar(
        &mut rgba,
        SIZE,
        RIGHT_X,
        BAR_Y,
        MAIN_BAR_W,
        BAR_H,
        parse_percent(&metrics.weekly),
    );
    draw_reset_bar(
        &mut rgba,
        SIZE,
        RIGHT_X + GROUP_W - RESET_BAR_W,
        BAR_Y,
        RESET_BAR_W,
        BAR_H,
        metrics.weekly_reset,
    );

    vec![ksni::Icon {
        width: SIZE as i32,
        height: SIZE as i32,
        data: rgba_to_argb(&rgba),
    }]
}

fn percent_string(value: Option<f64>) -> String {
    value
        .map(|pct| format!("{:.0}%", pct.round().clamp(0.0, 100.0)))
        .unwrap_or_else(|| String::from("%?"))
}

pub fn replace_existing_host(socket_path: &Path) -> Result<bool, String> {
    let Ok(mut stream) = UnixStream::connect(socket_path) else {
        return Ok(false);
    };
    stream
        .write_all(b"quit\n")
        .map_err(|err| format!("Failed to ask existing tray instance to quit: {err}"))?;

    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(50));
        if !socket_path.exists() {
            return Ok(true);
        }
        if UnixStream::connect(socket_path).is_err() {
            return Ok(true);
        }
    }

    // Best-effort handoff: the old instance may still be in the middle of
    // shutting down, but the new process should continue and take over.
    Ok(true)
}

struct PopupManager {
    popup_child: Option<Child>,
    options: TrayOptions,
    socket_path: PathBuf,
}

impl PopupManager {
    fn new(options: TrayOptions, socket_path: PathBuf) -> Self {
        Self {
            popup_child: None,
            options,
            socket_path,
        }
    }

    fn toggle(&mut self, position: Option<(i32, i32)>) {
        self.cleanup_dead_child();
        if self.popup_child.is_some() {
            self.kill_popup();
        } else if let Err(err) = self.spawn_popup(position) {
            eprintln!("Failed to open popup: {err}");
        }
    }

    fn show(&mut self, position: Option<(i32, i32)>) {
        self.cleanup_dead_child();
        if self.popup_child.is_none() {
            if let Err(err) = self.spawn_popup(position) {
                eprintln!("Failed to open popup: {err}");
            }
        }
    }

    fn shutdown(&mut self) {
        self.kill_popup();
        let _ = fs::remove_file(&self.socket_path);
    }

    fn cleanup_dead_child(&mut self) {
        if let Some(child) = self.popup_child.as_mut() {
            if child.try_wait().ok().flatten().is_some() {
                self.popup_child = None;
            }
        }
    }

    fn kill_popup(&mut self) {
        if let Some(mut child) = self.popup_child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn spawn_popup(&mut self, position: Option<(i32, i32)>) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|err| err.to_string())?;
        let mut command = Command::new(exe);
        command.arg("--popup");

        if let Some(browser) = self.options.browser {
            command.arg("--browser").arg(browser.to_string());
        }
        if let Some(data_dir) = &self.options.data_dir {
            command.arg("--data-dir").arg(data_dir);
        }
        if let Some(oauth_dir) = &self.options.oauth_dir {
            command.arg("--oauth-dir").arg(oauth_dir);
        }
        if let Some(title) = &self.options.title {
            command.arg("--title").arg(title);
        }

        if let Some((x, y)) = position {
            let (popup_x, popup_y) = popup_origin(x, y);
            command
                .arg("--popup-x")
                .arg(popup_x.to_string())
                .arg("--popup-y")
                .arg(popup_y.to_string());
        }

        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let child = command.spawn().map_err(|err| err.to_string())?;
        self.popup_child = Some(child);
        Ok(())
    }
}

fn popup_origin(x: i32, y: i32) -> (i32, i32) {
    let popup_x = (x - POPUP_WIDTH + POPUP_MARGIN).max(0);
    let popup_y = if y < POPUP_HEIGHT {
        y + POPUP_MARGIN
    } else {
        y - POPUP_HEIGHT - POPUP_MARGIN
    };
    (popup_x, popup_y.max(0))
}

fn bind_socket(socket_path: &Path) -> Result<UnixListener, String> {
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }

    match UnixListener::bind(socket_path) {
        Ok(listener) => Ok(listener),
        Err(first_err) => {
            let _ = fs::remove_file(socket_path);
            UnixListener::bind(socket_path).map_err(|_| first_err.to_string())
        }
    }
}

fn start_socket_listener(listener: UnixListener, manager: Arc<Mutex<PopupManager>>) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };

            let mut command = String::new();
            if stream.read_to_string(&mut command).is_err() {
                continue;
            }

            match command.trim() {
                "toggle" => manager.lock().unwrap().toggle(None),
                "quit" => {
                    manager.lock().unwrap().shutdown();
                    std::process::exit(0);
                }
                _ => {}
            }
        }
    });
}

struct ClaudeUsageTray {
    manager: Arc<Mutex<PopupManager>>,
    metrics: TrayMetrics,
}

impl ClaudeUsageTray {
    fn toggle(&self, position: Option<(i32, i32)>) {
        self.manager.lock().unwrap().toggle(position);
    }

    fn show(&self, position: Option<(i32, i32)>) {
        self.manager.lock().unwrap().show(position);
    }

    fn quit(&self) {
        self.manager.lock().unwrap().shutdown();
        std::process::exit(0);
    }
}

impl ksni::Tray for ClaudeUsageTray {
    fn id(&self) -> String {
        String::from("claude-usage")
    }

    fn title(&self) -> String {
        self.metrics.title.clone()
    }

    fn icon_name(&self) -> String {
        String::new()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        tray_icon_pixmap(&self.metrics)
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: String::from("Claude Usage"),
            description: self.metrics.tooltip.clone(),
            ..Default::default()
        }
    }

    fn activate(&mut self, x: i32, y: i32) {
        self.toggle(Some((x, y)));
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            ksni::menu::StandardItem {
                label: self.metrics.title.clone(),
                enabled: false,
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            ksni::menu::StandardItem {
                label: String::from("Show Usage"),
                activate: Box::new(|tray: &mut Self| tray.show(None)),
                ..Default::default()
            }
            .into(),
            ksni::menu::StandardItem {
                label: String::from("Quit"),
                activate: Box::new(|tray: &mut Self| tray.quit()),
                ..Default::default()
            }
            .into(),
        ]
    }
}

fn resolve_tray_browser_and_cookies(
    options: &TrayOptions,
    config: &config::Config,
) -> (BrowserKind, Option<cookies::CookieJar>) {
    let initial_cookies = config.cached_cookies.clone();
    let cached_browser = config
        .cached_browser
        .as_deref()
        .and_then(|value| match value {
            "firefox" => Some(BrowserKind::Firefox),
            "chrome" => Some(BrowserKind::Chrome),
            "brave" => Some(BrowserKind::Brave),
            "edge" => Some(BrowserKind::Edge),
            _ => None,
        });
    let has_cached_session = initial_cookies
        .as_ref()
        .is_some_and(|cookies| cookies.contains_key("sessionKey"));
    let has_oauth = crate::oauth::read_access_token(options.oauth_dir.as_deref()).is_some();

    let browser = if let Some(browser) = options.browser {
        browser
    } else if has_cached_session {
        cached_browser.unwrap_or_else(detect_browser_or_fallback)
    } else if has_oauth {
        cached_browser
            .or_else(|| cookies::detect_browser("claude.ai"))
            .unwrap_or(BrowserKind::Firefox)
    } else {
        detect_browser_or_fallback()
    };

    (browser, initial_cookies)
}

fn detect_browser_or_fallback() -> BrowserKind {
    cookies::detect_browser("claude.ai").unwrap_or(BrowserKind::Firefox)
}

fn start_usage_updater(handle: ksni::Handle<ClaudeUsageTray>, options: TrayOptions) {
    std::thread::spawn(move || {
        let config = config::Config::load();
        let (browser, initial_cookies) = resolve_tray_browser_and_cookies(&options, &config);
        let mut usage = UsageModel::new(
            browser,
            options.data_dir.clone(),
            options.oauth_dir.clone(),
            &config,
            initial_cookies,
        );

        if let Some(Ok(snapshot)) = usage.cached_data.as_ref() {
            let metrics = TrayMetrics::from_usage(snapshot);
            handle.update(|tray| {
                tray.metrics = metrics;
            });
        }

        loop {
            if usage.should_refresh() {
                usage.start_fetch(false);
            }

            usage.poll_result();
            if let Some(Ok(snapshot)) = usage.cached_data.as_ref() {
                let metrics = TrayMetrics::from_usage(snapshot);
                handle.update(|tray| {
                    tray.metrics = metrics;
                });
            }

            std::thread::sleep(TRAY_POLL_INTERVAL);
        }
    });
}

pub fn run_tray_host(options: TrayOptions) -> Result<(), String> {
    let socket_path = config::tray_socket_path()
        .ok_or_else(|| String::from("Could not determine tray socket path"))?;
    let listener = bind_socket(&socket_path)?;
    let manager = Arc::new(Mutex::new(PopupManager::new(options.clone(), socket_path)));
    start_socket_listener(listener, Arc::clone(&manager));

    let service = ksni::TrayService::new(ClaudeUsageTray {
        manager,
        metrics: TrayMetrics::default(),
    });
    let handle = service.handle();
    start_usage_updater(handle, options);
    service.spawn();

    loop {
        std::thread::park();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::api::UsageBucket;

    use super::*;

    #[test]
    fn tray_metrics_format_primary_quotas() {
        let mut usage = HashMap::new();
        usage.insert(
            String::from("five_hour"),
            UsageBucket {
                utilization: Some(41.6),
                resets_at: Some((chrono::Utc::now() + chrono::TimeDelta::hours(2)).to_rfc3339()),
            },
        );
        usage.insert(
            String::from("seven_day"),
            UsageBucket {
                utilization: Some(12.4),
                resets_at: Some((chrono::Utc::now() + chrono::TimeDelta::days(3)).to_rfc3339()),
            },
        );

        let metrics = TrayMetrics::from_usage(&usage);

        assert_eq!(metrics.title, "5h 42% | 7d 12%");
        assert!(metrics.tooltip.contains("5h: 42%"));
        assert!(metrics.tooltip.contains("7d: 12%"));
        assert!(metrics.five_hour_reset.is_some());
        assert!(metrics.weekly_reset.is_some());
    }
}
