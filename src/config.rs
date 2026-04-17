use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub always_on_top: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub all_workspaces: Option<bool>,
    /// Exe path that last showed the Chromium permission explanation dialog.
    /// Stored so we don't re-show the dialog on subsequent launches from the
    /// same binary (macOS ties "Always Allow" to the binary path).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chromium_prompted_exe: Option<String>,
    /// Cached browser cookies so we don't need to re-read the browser DB
    /// and re-decrypt on every launch.  Re-read only when the API rejects them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_cookies: Option<std::collections::HashMap<String, String>>,
    /// Which browser the cached cookies came from (for fallback re-reads).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_browser: Option<String>,
    /// Last successful usage response for fast popup startup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_usage_snapshot: Option<crate::api::UsageResponse>,
    /// UNIX timestamp in seconds for the last successful usage response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_usage_fetched_at: Option<i64>,
}

impl Config {
    pub fn load() -> Self {
        config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = config_path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

pub fn config_root_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("SmartAppsCo/claude-usage-widget"))
}

pub fn config_path() -> Option<PathBuf> {
    config_root_dir().map(|d| d.join("config.json"))
}

#[cfg(target_os = "linux")]
pub fn tray_socket_path() -> Option<PathBuf> {
    config_root_dir().map(|d| d.join("tray.sock"))
}

#[cfg(target_os = "linux")]
fn config_dir() -> Option<PathBuf> {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .ok()
        .or_else(|| crate::cookies::platform::home_dir().map(|h| h.join(".config")))
}

#[cfg(target_os = "macos")]
fn config_dir() -> Option<PathBuf> {
    crate::cookies::platform::home_dir().map(|h| h.join("Library/Application Support"))
}

#[cfg(target_os = "windows")]
fn config_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::api::UsageBucket;

    use super::*;

    #[test]
    fn old_config_fields_still_deserialize() {
        let json = r#"{
            "refresh_secs": 300,
            "always_on_top": true,
            "all_workspaces": true
        }"#;

        let config: Config = serde_json::from_str(json).unwrap();

        assert_eq!(config.refresh_secs, Some(300));
        assert_eq!(config.always_on_top, Some(true));
        assert_eq!(config.all_workspaces, Some(true));
        assert!(config.last_usage_snapshot.is_none());
    }

    #[test]
    fn snapshot_fields_round_trip() {
        let mut snapshot = HashMap::new();
        snapshot.insert(
            String::from("five_hour"),
            UsageBucket {
                utilization: Some(17.0),
                resets_at: Some(String::from("2026-04-17T10:00:00Z")),
            },
        );

        let config = Config {
            last_usage_snapshot: Some(snapshot),
            last_usage_fetched_at: Some(1234),
            ..Default::default()
        };

        let json = serde_json::to_string(&config).unwrap();
        let decoded: Config = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.last_usage_fetched_at, Some(1234));
        assert!(decoded.last_usage_snapshot.is_some());
    }
}
