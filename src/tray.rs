#![cfg(target_os = "linux")]

use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use crate::config;
use crate::cookies::BrowserKind;

const POPUP_WIDTH: i32 = 186;
const POPUP_HEIGHT: i32 = 274;
const POPUP_MARGIN: i32 = 12;

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

pub fn signal_existing_host(socket_path: &Path) -> bool {
    let Ok(mut stream) = UnixStream::connect(socket_path) else {
        return false;
    };
    stream.write_all(b"toggle\n").is_ok()
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
                "toggle" => {
                    manager.lock().unwrap().toggle(None);
                }
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
        String::from("Claude Usage")
    }

    fn icon_name(&self) -> String {
        String::from("claude-usage")
    }

    fn activate(&mut self, x: i32, y: i32) {
        self.toggle(Some((x, y)));
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
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

pub fn run_tray_host(options: TrayOptions) -> Result<(), String> {
    let socket_path = config::tray_socket_path()
        .ok_or_else(|| String::from("Could not determine tray socket path"))?;
    let listener = bind_socket(&socket_path)?;
    let manager = Arc::new(Mutex::new(PopupManager::new(options, socket_path)));
    start_socket_listener(listener, Arc::clone(&manager));

    let service = ksni::TrayService::new(ClaudeUsageTray { manager });
    let _handle = service.spawn();

    loop {
        std::thread::park();
    }
}
