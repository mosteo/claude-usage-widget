# Claude Usage

Claude's usage page is buried in settings and requires you to open your browser every time you want to check how much of your plan you've used. On Linux, this app lives in your system tray and opens a lightweight popup with your current session and weekly usage at a glance — pulled from the same data shown at https://claude.ai/settings/usage — so you always know where you stand before hitting a rate limit.

Especially useful for heavy Claude Code users who burn through their allocation quickly and want to keep an eye on remaining capacity without breaking their workflow.

<p align="center">
  <img src="images/screenshot.png" alt="claude-usage-widget screenshot" width="200">
</p>

## Prerequisites

- **Claude Code** users: works automatically — the app reads your existing OAuth credentials from `~/.claude/.credentials.json` with no extra setup
- **Browser-only** users: log into claude.ai in a supported browser (Firefox, Chrome, Brave, Edge, or Safari)

The app tries Claude Code OAuth credentials first, then falls back to reading browser session cookies. No API key needed.

## Platform Notes

The widget auto-detects your browser and reads cookies directly. Depending on the platform and browser, you may see a one-time permission prompt:

| Platform | Browser | Prompt |
|---|---|---|
| **macOS** | Firefox | None |
| **macOS** | Chrome, Brave, Edge | macOS Keychain password (click "Always Allow" to avoid repeat prompts) |
| **macOS** | Safari | Requires Full Disk Access in System Settings → Privacy & Security |
| **Windows** | Chrome, Edge, Brave | UAC elevation prompt (needed to release cookie DB locks and decrypt App-Bound Encryption) |
| **Windows** | Firefox | None |
| **Linux** | All | None |

Browsers are tried in order until a valid session is found. On macOS, Firefox is tried first to avoid unnecessary prompts. Use `--browser` to skip auto-detection and target a specific browser.

## Install

Download the latest binary for your platform from the [Releases](https://github.com/SmartAppsCo/claude-usage-widget/releases) page:

| Platform | File |
|---|---|
| Linux (x86_64) | `claude-usage-x86_64-unknown-linux-gnu.tar.gz` |
| macOS (Apple Silicon) | `claude-usage-aarch64-apple-darwin.zip` |
| macOS (Intel) | `claude-usage-x86_64-apple-darwin.zip` |
| Windows (x86_64) | `claude-usage-x86_64-pc-windows-msvc.zip` |

Extract the archive. On macOS, double-click `Claude Usage.app` to run (or drag it to Applications). On Linux/Windows, place the binary somewhere in your `PATH`.

Alternatively, build from source with [Cargo](https://rustup.rs/):

```
cargo install --git https://github.com/SmartAppsCo/claude-usage-widget.git --tag v0.7.0
```

On Linux, building from source requires these system packages:

```
sudo apt install libgtk-3-dev libxcb-screensaver0-dev
```

## Options

```
--browser <BROWSER>                Browser to read cookies from (auto-detected if omitted)
                                   (firefox, chrome, brave, edge, safari*) (* macOS only)
--data-dir <PATH>                  Custom browser data directory (requires --browser)
--oauth-dir <PATH>                 Claude Code credentials directory (default: ~/.claude)
--title <NAME>                     Display name shown in the popup header
```

`--data-dir` is useful for non-standard browser installations or custom profiles where the cookie database isn't in the default location.

`--oauth-dir` is useful if you have multiple Claude Code configurations (e.g. `~/.claude-work`).

When `--title` is omitted, the popup fetches your name from the "What should Claude call you?" setting on your account. You can change this at https://claude.ai/settings/general. Passing `--title` skips that extra API call.

## Behavior

- On Linux, launching the app starts a tray icon. Left-click it to toggle the usage popup.
- The popup closes automatically when it loses focus, and you can still use `Esc` or the close button.
- Usage data is cached after a successful fetch so the popup can open immediately with the last known snapshot, then refresh in the background.
- The popup polls usage data every 5 minutes by default and pauses polling automatically when you're idle (no keyboard/mouse activity). Right-click the popup to adjust the refresh interval.

## Disclaimer

This project is not affiliated with, endorsed by, or associated with Anthropic, PBC. "Claude" and "Anthropic" are trademarks of Anthropic, PBC. All other trademarks are the property of their respective owners. This is an independent, unofficial tool.
