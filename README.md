# 🟠 status-overlay

> Claude usage API integration based on [CodexBar](https://github.com/steipete/CodexBar) by Peter Steinberger (steipete).
> IPC daemon pattern inspired by [hyprvoice](https://github.com/leonardotrapani/hyprvoice) by Leonardo Trapani.

A floating Wayland overlay built in Rust with GTK4 and layer-shell. Shows the time, date, calendar, and live Claude AI usage stats — all in a blurred, translucent panel that floats above everything without stealing focus.

## ✨ Features

- **⏰ Live clock** — seconds-accurate, updates every tick
- **📅 Calendar** — built-in GTK4 month view
- **🤖 Claude usage** — pulls live 5-hour session and 7-day weekly utilization from the Anthropic OAuth API
- **📊 Today's stats** — message and tool-call counts from local Claude Code stats cache
- **💳 Extra usage** — shows Extra tier spend vs. monthly limit
- **🎨 Compositor blur** — Hyprland `layerrule` blur on the `status-overlay` namespace
- **🔌 Unix socket IPC** — show, hide, toggle, quit from any script or keybind
- **🦀 Pure Rust** — no eww, no scripts, direct GTK4

## 🛠️ Stack

- **Rust** + **GTK4** — direct UI, no middleware
- **gtk4-layer-shell** — Wayland layer-shell protocol
- **ureq** + **serde_json** — lightweight sync HTTP for the Anthropic API
- **chrono** — time formatting
- **Nix flakes** — crane + fenix for reproducible builds

## 🚀 Usage

```bash
# Run daemon (window starts visible)
nix run

# Or in dev shell
nix develop --command cargo run
```

### IPC commands

Once the daemon is running, control it from any terminal or keybind:

```bash
status-overlay toggle   # show if hidden, hide if visible
status-overlay show     # bring to foreground
status-overlay hide     # send to background
status-overlay refresh  # ask both usage sections to refresh now
status-overlay quit     # kill the daemon
status-overlay --help   # show CLI help
```

The socket lives at `$XDG_RUNTIME_DIR/status-overlay.sock`.

Last successful fetches are cached to disk at `$XDG_STATE_HOME/status-overlay/{usage,codex}.json` (falls back to `~/.local/state`). Every successful Claude and Codex sample is also appended to `$XDG_STATE_HOME/status-overlay/history.sqlite3`, and legacy `usage_history.json` / `codex_history.json` files are imported into that database automatically on first open. Cached data is shown as “stale” on startup until a fresh fetch succeeds.

Press `q` while the overlay has focus to hide it.

## ⌨️ Hyprland integration

Add to your Hyprland config (or home-manager):

```nix
# Start the daemon on login
exec-once = [ "status-overlay" ];

# Toggle with a keybind
bind = [ "$mod, O, exec, status-overlay toggle" ];
```

## 🌫️ Blur setup

Add to your Hyprland layerrule config:

```nix
layerrule = [
  "blur, status-overlay"
  "ignorealpha 0.1, status-overlay"
];
```

`ignorealpha 0.1` prevents blurring fully transparent pixels, keeping the rounded corners clean.

## 🤖 Claude + Codex usage

- **Claude:** polls every 5 minutes but also refreshes whenever the window is shown or a `refresh`/`show`/`toggle` IPC arrives, with a minimum 30-second gap between fetches.
- **Codex:** polls every 60 seconds and also refreshes on `refresh`/`show`/`toggle`.

Desktop notifications fire via `notify-send` when:

| Event | Threshold |
|-------|-----------|
| Low warning | ≥ 90% used |
| Depleted | ≥ 100% / limit reached |
| Restored | drops back below 30% |

### Claude



Reads from two local sources — no extra config needed if you use Claude Code:

| Source | Data |
|--------|------|
| `~/.claude/.credentials.json` | OAuth token for the Anthropic API |
| `~/.claude/stats-cache.json` | Today's message + tool-call counts |

Live usage (session %, weekly %, extra spend) is fetched from `https://api.anthropic.com/api/oauth/usage`.

### Codex

| Source | Data |
|--------|------|
| `~/.codex/auth.json` | OAuth token (or `OPENAI_API_KEY`) |

Live usage (5h session %, 7d weekly %, plan type) is fetched from `https://chatgpt.com/backend-api/wham/usage`.

## 📄 License

MIT — see [LICENSE](LICENSE)
