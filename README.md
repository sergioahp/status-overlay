# 🟠 status-overlay

A floating Wayland overlay built in Rust with GTK4 and layer-shell. Shows the time, date, calendar, and live Claude AI usage stats — all in a blurred, translucent panel that floats above everything without stealing focus.

## ✨ Features

- **⏰ Live clock** — seconds-accurate, updates every tick
- **📅 Calendar** — built-in GTK4 month view
- **🤖 Claude usage** — pulls live 5-hour session and 7-day weekly utilization from the Anthropic OAuth API
- **📊 Today's stats** — message and tool-call counts from local Claude Code stats cache
- **💳 Extra usage** — shows Extra tier spend vs. monthly limit
- **🎨 Compositor blur** — Hyprland `layerrule` blur on the `status-overlay` namespace
- **⌨️ Press `q` to dismiss**
- **🦀 Pure Rust** — no eww, no scripts, direct GTK4

## 🛠️ Stack

- **Rust** + **GTK4** — direct UI, no middleware
- **gtk4-layer-shell** — Wayland layer-shell protocol
- **ureq** + **serde_json** — lightweight sync HTTP for the Anthropic API
- **chrono** — time formatting
- **Nix flakes** — crane + fenix for reproducible builds

## 🚀 Usage

```bash
# Run in dev shell
nix develop --command cargo run

# Build store binary (with wrapGAppsHook, works outside dev shell)
nix build
./result/bin/status-overlay

# Or directly
nix run
```

Press `q` to close the overlay.

## 🔧 Hyprland blur setup

Add to your Hyprland config (or home-manager):

```nix
layerrule = [
  "blur, status-overlay"
  "ignorealpha 0.1, status-overlay"
];
```

## 🤖 Claude usage data

Reads from two local sources — no extra config needed if you use Claude Code:

| Source | Data |
|--------|------|
| `~/.claude/.credentials.json` | OAuth token for the Anthropic API |
| `~/.claude/stats-cache.json` | Today's message + tool-call counts |

Live usage (session %, weekly %, extra spend) is fetched from `https://api.anthropic.com/api/oauth/usage` and refreshed every 60 seconds.

## 📄 License

MIT — see [LICENSE](LICENSE)
