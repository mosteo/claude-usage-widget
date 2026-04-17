use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::api::{self, UsageResponse};
use crate::config::Config;
use crate::cookies::{self, BrowserKind, CookieJar};

const DEFAULT_REFRESH_SECS: u64 = 300;
const IDLE_THRESHOLD_SECS: u64 = 60;

struct SharedState {
    data: Option<Result<UsageResponse, String>>,
    fetching: bool,
    account_name: Option<String>,
    fresh_cookies: Option<CookieJar>,
}

pub struct PollOutcome {
    pub account_name: Option<String>,
    pub received_first_success: bool,
}

pub struct UsageModel {
    browser: BrowserKind,
    data_dir: Option<String>,
    oauth_dir: Option<String>,
    shared: Arc<Mutex<SharedState>>,
    pub cached_data: Option<Result<UsageResponse, String>>,
    cached_cookies: Option<CookieJar>,
    pub last_fetch: Option<SystemTime>,
    last_fetch_start: Option<Instant>,
    pub refresh_secs: u64,
}

impl UsageModel {
    pub fn new(
        browser: BrowserKind,
        data_dir: Option<String>,
        oauth_dir: Option<String>,
        config: &Config,
        initial_cookies: Option<CookieJar>,
    ) -> Self {
        let cached_data = config.last_usage_snapshot.clone().map(Ok);
        let last_fetch = config
            .last_usage_fetched_at
            .map(|secs| UNIX_EPOCH + Duration::from_secs(secs as u64));

        Self {
            browser,
            data_dir,
            oauth_dir,
            shared: Arc::new(Mutex::new(SharedState {
                data: None,
                fetching: false,
                account_name: None,
                fresh_cookies: None,
            })),
            cached_data,
            cached_cookies: initial_cookies,
            last_fetch,
            last_fetch_start: None,
            refresh_secs: config.refresh_secs.unwrap_or(DEFAULT_REFRESH_SECS),
        }
    }

    pub fn set_refresh_secs(&mut self, refresh_secs: u64) {
        self.refresh_secs = refresh_secs;
        let mut config = Config::load();
        config.refresh_secs = Some(refresh_secs);
        config.save();
    }

    pub fn start_fetch(&mut self, need_account_name: bool) {
        {
            let mut shared = self.shared.lock().unwrap();
            if shared.fetching {
                return;
            }
            shared.fetching = true;
        }

        self.last_fetch_start = Some(Instant::now());
        let shared = Arc::clone(&self.shared);
        let browser = self.browser;
        let data_dir = self.data_dir.clone();
        let oauth_dir = self.oauth_dir.clone();
        let cached_cookies = self.cached_cookies.clone();

        std::thread::spawn(move || {
            let (result, account_name, fresh_cookies) = Self::fetch_with_fallback(
                cached_cookies,
                browser,
                data_dir.as_deref(),
                oauth_dir.as_deref(),
                need_account_name,
            );

            let mut state = shared.lock().unwrap();
            state.data = Some(result);
            state.account_name = account_name;
            state.fresh_cookies = fresh_cookies;
            state.fetching = false;
        });
    }

    pub fn poll_result(&mut self) -> PollOutcome {
        let mut outcome = PollOutcome {
            account_name: None,
            received_first_success: false,
        };

        let mut shared = self.shared.lock().unwrap();
        if let Some(name) = shared.account_name.take() {
            outcome.account_name = Some(name);
        }

        if let Some(jar) = shared.fresh_cookies.take() {
            self.cached_cookies = Some(jar.clone());
            let mut config = Config::load();
            config.cached_cookies = Some(jar);
            config.cached_browser = Some(self.browser.to_string());
            config.save();
        }

        if let Some(result) = shared.data.take() {
            match result {
                Ok(data) => {
                    let had_success = matches!(self.cached_data, Some(Ok(_)));
                    self.cached_data = Some(Ok(data.clone()));
                    self.last_fetch = Some(SystemTime::now());
                    if !had_success {
                        outcome.received_first_success = true;
                    }

                    let mut config = Config::load();
                    config.last_usage_snapshot = Some(data);
                    config.last_usage_fetched_at = self
                        .last_fetch
                        .and_then(|ts| ts.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64);
                    config.save();
                }
                Err(err) => {
                    if self.cached_data.is_none()
                        || self.cached_data.as_ref().is_some_and(|data| data.is_err())
                    {
                        self.cached_data = Some(Err(err));
                    }
                }
            }
        }

        outcome
    }

    pub fn should_refresh(&self) -> bool {
        if let Some(idle) = crate::idle::system_idle_secs() {
            if idle > IDLE_THRESHOLD_SECS {
                return false;
            }
        }

        match self.last_fetch_start {
            Some(started_at) => started_at.elapsed() >= Duration::from_secs(self.refresh_secs),
            None => true,
        }
    }

    fn fetch_with_fallback(
        cached: Option<CookieJar>,
        browser: BrowserKind,
        data_dir: Option<&str>,
        oauth_dir: Option<&str>,
        need_name: bool,
    ) -> (
        Result<UsageResponse, String>,
        Option<String>,
        Option<CookieJar>,
    ) {
        if let Some(token) = crate::oauth::read_access_token(oauth_dir) {
            if let Ok((data, email)) = api::fetch_with_oauth(&token) {
                let name = if need_name { email } else { None };
                return (Ok(data), name, None);
            }
        }

        if let Some(ref jar) = cached {
            if let Ok(data) = api::fetch_with_cookies(jar) {
                let name = if need_name {
                    api::fetch_account_name(jar).ok()
                } else {
                    None
                };
                return (Ok(data), name, None);
            }
        }

        let jar = match cookies::read_cookies(browser, "claude.ai", data_dir) {
            Ok(jar) => jar,
            Err(_) => {
                #[cfg(target_os = "windows")]
                if matches!(
                    browser,
                    BrowserKind::Chrome | BrowserKind::Brave | BrowserKind::Edge
                ) {
                    let msg = if cached.is_some() {
                        "Your Claude session cookie has expired. Claude Usage needs \
                         administrator access to re-read cookies from the browser.\n\n\
                         The app will restart with the required permissions."
                    } else {
                        "Chrome, Edge, and Brave lock their cookie databases and use \
                         App-Bound Encryption. Claude Usage needs administrator access \
                         to read and decrypt them.\n\n\
                         Windows will prompt for permission next."
                    };
                    cookies::platform::elevate_with_message(msg);
                }

                return (
                    Err("Could not read cookies from browser".into()),
                    None,
                    None,
                );
            }
        };

        let result = api::fetch_with_cookies(&jar);
        let name = if need_name {
            api::fetch_account_name(&jar).ok()
        } else {
            None
        };
        let fresh = if result.is_ok() { Some(jar) } else { None };
        (result, name, fresh)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::api::UsageBucket;

    use super::*;

    #[test]
    fn loads_cached_snapshot_from_config() {
        let mut snapshot = HashMap::new();
        snapshot.insert(
            String::from("five_hour"),
            UsageBucket {
                utilization: Some(42.0),
                resets_at: Some(String::from("2026-04-17T10:00:00Z")),
            },
        );

        let config = Config {
            last_usage_snapshot: Some(snapshot),
            last_usage_fetched_at: Some(1_713_312_000),
            ..Default::default()
        };

        let model = UsageModel::new(BrowserKind::Firefox, None, None, &config, None);

        assert!(matches!(model.cached_data, Some(Ok(_))));
        assert!(model.last_fetch.is_some());
    }
}
