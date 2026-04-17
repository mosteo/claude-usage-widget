#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod api;
mod config;
mod cookies;
mod idle;
mod oauth;
#[cfg(target_os = "linux")]
mod tray;
mod usage_state;
mod widget;

use cookies::BrowserKind;

#[derive(Clone, Debug, PartialEq, Eq)]
struct CliOptions {
    browser: Option<BrowserKind>,
    data_dir: Option<String>,
    oauth_dir: Option<String>,
    title: String,
    title_explicit: bool,
    uninstall: bool,
    popup: bool,
    popup_x: Option<i32>,
    popup_y: Option<i32>,
}

impl Default for CliOptions {
    fn default() -> Self {
        Self {
            browser: None,
            data_dir: None,
            oauth_dir: None,
            title: String::from("Plan Usage"),
            title_explicit: false,
            uninstall: false,
            popup: false,
            popup_x: None,
            popup_y: None,
        }
    }
}

#[cfg(target_os = "linux")]
fn xdg_data_dir() -> std::path::PathBuf {
    std::env::var("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            cookies::platform::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                .join(".local/share")
        })
}

#[cfg(target_os = "linux")]
fn desktop_file_path() -> std::path::PathBuf {
    xdg_data_dir().join("applications/claude-usage.desktop")
}

#[cfg(target_os = "linux")]
fn icon_install_path() -> std::path::PathBuf {
    xdg_data_dir().join("icons/hicolor/256x256/apps/claude-usage.png")
}

#[cfg(target_os = "linux")]
fn install_desktop_entry() {
    let desktop_path = desktop_file_path();
    let icon_path = icon_install_path();

    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_else(|| String::from("claude-usage"));

    if desktop_path.exists() && icon_path.exists() {
        if let Ok(contents) = std::fs::read_to_string(&desktop_path) {
            if contents.contains(&format!("Exec={exe}")) {
                return;
            }
        }
    }

    let desktop_content = format!(
        "[Desktop Entry]
Type=Application
Name=Claude Usage
Comment=Tray app for Claude usage stats
Exec={exe}
Icon=claude-usage
Terminal=false
StartupWMClass=claude-usage
StartupNotify=false
Categories=Utility;
"
    );

    if let Some(parent) = desktop_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&desktop_path, desktop_content);

    if let Some(parent) = icon_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&icon_path, include_bytes!("../images/icon.png"));

    let _ = std::process::Command::new("gtk-update-icon-cache")
        .args(["-f", "-t"])
        .arg(
            icon_path
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap(),
        )
        .output();
    let _ = std::process::Command::new("update-desktop-database")
        .arg(desktop_path.parent().unwrap())
        .output();
}

#[cfg(target_os = "linux")]
fn uninstall_desktop_entry() {
    let _ = std::fs::remove_file(desktop_file_path());
    let _ = std::fs::remove_file(icon_install_path());
    eprintln!("Desktop entry and icon removed.");
}

fn fatal_error(msg: &str) -> ! {
    eprintln!("{msg}");
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::WindowsAndMessaging::*;
        use windows::core::PCWSTR;
        let text: Vec<u16> = msg.encode_utf16().chain(std::iter::once(0)).collect();
        let caption: Vec<u16> = "Claude Usage\0".encode_utf16().collect();
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(text.as_ptr()),
                PCWSTR(caption.as_ptr()),
                MB_OK | MB_ICONERROR,
            );
        }
    }
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display dialog {:?} buttons {{\"OK\"}} default button \"OK\" with icon stop with title \"Claude Usage\"",
            msg
        );
        let _ = std::process::Command::new("osascript")
            .args(["-e", &script])
            .output();
    }
    #[cfg(target_os = "linux")]
    {
        let shown = std::process::Command::new("zenity")
            .args(["--error", "--title=Claude Usage", "--text", msg])
            .output()
            .is_ok_and(|o| o.status.success())
            || std::process::Command::new("kdialog")
                .args(["--error", msg, "--title", "Claude Usage"])
                .output()
                .is_ok_and(|o| o.status.success());
        if !shown {
            let _ = std::process::Command::new("notify-send")
                .args(["--urgency=critical", "Claude Usage", msg])
                .output();
        }
    }
    std::process::exit(1);
}

fn print_usage() {
    eprintln!("Usage: claude-usage [OPTIONS]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --browser <BROWSER>         Browser to read cookies from");
    eprintln!("                              (firefox, chrome, brave, edge, safari)");
    eprintln!("  --data-dir <PATH>           Browser data directory");
    eprintln!("  --oauth-dir <PATH>          Claude Code credentials directory");
    eprintln!("                              (default: ~/.claude)");
    eprintln!("  --title <NAME>              Popup title (default: Plan Usage)");
    eprintln!("  --uninstall                 Remove desktop entry and icon");
    eprintln!("  --help                      Show this help");
}

fn parse_browser(value: &str) -> Result<BrowserKind, String> {
    match value {
        "firefox" => Ok(BrowserKind::Firefox),
        "chrome" => Ok(BrowserKind::Chrome),
        "brave" => Ok(BrowserKind::Brave),
        "edge" => Ok(BrowserKind::Edge),
        #[cfg(target_os = "macos")]
        "safari" => Ok(BrowserKind::Safari),
        _ => Err(format!("Error: unknown browser '{value}'")),
    }
}

fn parse_i32_arg(flag: &str, value: &str) -> Result<i32, String> {
    value
        .parse::<i32>()
        .map_err(|_| format!("Error: {flag} requires an integer value"))
}

fn parse_args(args: &[String]) -> Result<CliOptions, String> {
    let mut options = CliOptions::default();
    let mut i = 0usize;

    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            "--uninstall" => options.uninstall = true,
            "--popup" => options.popup = true,
            "--browser" => {
                i += 1;
                if i >= args.len() {
                    return Err(String::from("Error: --browser requires a value"));
                }
                options.browser = Some(parse_browser(&args[i])?);
            }
            "--data-dir" => {
                i += 1;
                if i >= args.len() {
                    return Err(String::from("Error: --data-dir requires a value"));
                }
                options.data_dir = Some(args[i].clone());
            }
            "--oauth-dir" => {
                i += 1;
                if i >= args.len() {
                    return Err(String::from("Error: --oauth-dir requires a value"));
                }
                options.oauth_dir = Some(args[i].clone());
            }
            "--title" => {
                i += 1;
                if i >= args.len() {
                    return Err(String::from("Error: --title requires a value"));
                }
                options.title = args[i].clone();
                options.title_explicit = true;
            }
            "--popup-x" => {
                i += 1;
                if i >= args.len() {
                    return Err(String::from("Error: --popup-x requires a value"));
                }
                options.popup_x = Some(parse_i32_arg("--popup-x", &args[i])?);
            }
            "--popup-y" => {
                i += 1;
                if i >= args.len() {
                    return Err(String::from("Error: --popup-y requires a value"));
                }
                options.popup_y = Some(parse_i32_arg("--popup-y", &args[i])?);
            }
            other => return Err(format!("Error: unknown argument '{other}'")),
        }
        i += 1;
    }

    if options.data_dir.is_some() && options.browser.is_none() {
        return Err(String::from("Error: --data-dir requires --browser"));
    }
    if (options.popup_x.is_some() || options.popup_y.is_some()) && !options.popup {
        return Err(String::from("Error: --popup-x/--popup-y require --popup"));
    }

    Ok(options)
}

fn detect_browser_or_exit() -> BrowserKind {
    cookies::detect_browser("claude.ai")
        .unwrap_or_else(|| fatal_error("No claude.ai session found in any supported browser."))
}

fn resolve_browser_and_cookies(
    options: &CliOptions,
    config: &config::Config,
) -> (BrowserKind, Option<cookies::CookieJar>) {
    let mut initial_cookies = config.cached_cookies.clone();
    let cached_browser = config.cached_browser.as_deref().and_then(|s| match s {
        "firefox" => Some(BrowserKind::Firefox),
        "chrome" => Some(BrowserKind::Chrome),
        "brave" => Some(BrowserKind::Brave),
        "edge" => Some(BrowserKind::Edge),
        #[cfg(target_os = "macos")]
        "safari" => Some(BrowserKind::Safari),
        _ => None,
    });

    let has_cached_session = initial_cookies
        .as_ref()
        .is_some_and(|cookies| cookies.contains_key("sessionKey"));
    let has_oauth = oauth::read_access_token(options.oauth_dir.as_deref()).is_some();

    let browser = if let Some(browser) = options.browser {
        if !has_cached_session {
            #[cfg(target_os = "windows")]
            if matches!(
                browser,
                BrowserKind::Chrome | BrowserKind::Brave | BrowserKind::Edge
            ) {
                if !cookies::platform::elevate_if_needed() {
                    std::process::exit(0);
                }
            }

            match cookies::read_cookies(browser, "claude.ai", options.data_dir.as_deref()) {
                Ok(cookies) if cookies.contains_key("sessionKey") => {
                    initial_cookies = Some(cookies);
                }
                Ok(_) if !has_oauth => {
                    fatal_error(&format!("No claude.ai session found in {browser}."))
                }
                Err(err) if !has_oauth => {
                    fatal_error(&format!("Error reading {browser} cookies: {err}"))
                }
                _ => {}
            }
        }
        browser
    } else if has_cached_session {
        cached_browser.unwrap_or_else(detect_browser_or_exit)
    } else if has_oauth {
        cached_browser
            .or_else(|| cookies::detect_browser("claude.ai"))
            .unwrap_or(BrowserKind::Firefox)
    } else {
        detect_browser_or_exit()
    };

    (browser, initial_cookies)
}

#[cfg(target_os = "linux")]
fn detach_on_linux() {
    unsafe extern "C" {
        fn fork() -> i32;
        fn setsid() -> i32;
    }

    unsafe {
        let pid = fork();
        if pid > 0 {
            std::process::exit(0);
        }
        if pid == 0 {
            setsid();
        }
    }
}

fn build_popup_viewport(options: &CliOptions) -> eframe::egui::ViewportBuilder {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../images/icon.png"))
        .expect("Failed to load icon");

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_decorations(false)
        .with_inner_size([186.0, 274.0])
        .with_always_on_top()
        .with_resizable(false)
        .with_icon(icon);

    if let (Some(x), Some(y)) = (options.popup_x, options.popup_y) {
        viewport = viewport.with_position([x as f32, y as f32]);
    }

    viewport
}

fn run_popup(options: CliOptions) {
    let config = config::Config::load();
    let (browser, initial_cookies) = resolve_browser_and_cookies(&options, &config);
    let viewport = build_popup_viewport(&options);

    use eframe::egui;

    let app = widget::UsageApp::new(
        browser,
        options.data_dir,
        options.oauth_dir,
        options.title,
        options.title_explicit,
        config,
        initial_cookies,
    );

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Claude Usage Popup",
        options,
        Box::new(|cc| {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                String::from("noto_sans"),
                egui::FontData::from_static(include_bytes!("../fonts/NotoSans-Regular.ttf")),
            );
            fonts.font_data.insert(
                String::from("noto_sans_bold"),
                egui::FontData::from_static(include_bytes!("../fonts/NotoSans-Bold.ttf")),
            );
            fonts
                .families
                .get_mut(&egui::FontFamily::Proportional)
                .unwrap()
                .insert(0, String::from("noto_sans"));
            fonts.families.insert(
                egui::FontFamily::Name("bold".into()),
                vec![String::from("noto_sans_bold")],
            );
            cc.egui_ctx.set_fonts(fonts);
            Ok(Box::new(app))
        }),
    )
    .expect("Failed to start eframe");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = parse_args(&args).unwrap_or_else(|err| fatal_error(&err));

    if options.uninstall {
        #[cfg(target_os = "linux")]
        uninstall_desktop_entry();
        #[cfg(not(target_os = "linux"))]
        eprintln!("--uninstall is only supported on Linux");
        return;
    }

    #[cfg(target_os = "linux")]
    if !options.popup {
        install_desktop_entry();
        if let Some(socket_path) = config::tray_socket_path() {
            if let Err(err) = tray::replace_existing_host(&socket_path) {
                fatal_error(&err);
            }
        }

        detach_on_linux();
        if let Err(err) = tray::run_tray_host(tray::TrayOptions::from_cli(&options)) {
            fatal_error(&err);
        }
        return;
    }

    run_popup(options);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_popup_args() {
        let args = vec![
            String::from("--popup"),
            String::from("--popup-x"),
            String::from("120"),
            String::from("--popup-y"),
            String::from("80"),
            String::from("--browser"),
            String::from("firefox"),
        ];

        let options = parse_args(&args).unwrap();

        assert!(options.popup);
        assert_eq!(options.popup_x, Some(120));
        assert_eq!(options.popup_y, Some(80));
        assert_eq!(options.browser, Some(BrowserKind::Firefox));
    }

    #[test]
    fn rejects_popup_position_without_popup_mode() {
        let args = vec![String::from("--popup-x"), String::from("120")];

        let err = parse_args(&args).unwrap_err();

        assert!(err.contains("--popup-x/--popup-y require --popup"));
    }
}
