# status-overlay — AI Assistant Context

## Project overview

GTK4 Wayland overlay daemon written in Rust. Shows a clock, calendar, and live
Claude API usage stats. Controlled via Unix socket IPC. Targets Hyprland with
layer-shell blur.

Key files:
- `src/main.rs` — GTK4 window, layout, clock, key handler, IPC wiring
- `src/usage.rs` — Anthropic OAuth API + local stats-cache reader
- `src/ipc.rs` — Unix socket server/client (`$XDG_RUNTIME_DIR/status-overlay.sock`)
- `flake.nix` — crane + fenix build, `wrapProgram` for runtime libs

## Build & run

```bash
nix develop --command cargo build   # dev build
nix develop --command cargo run     # run daemon
nix build                           # reproducible store build
nix run                             # run store build
status-overlay toggle               # IPC client
```

## Code style

- **Prefer `let-else` and early returns** over nested `if let` chains.
- **Handle every `Result` and `Option` explicitly.** No silent `.unwrap()` in
  paths that can fail at runtime. A `.unwrap()` should have a comment explaining
  why it cannot fail.
- **Keep related logic together.** Prefer locality of behavior over aggressive
  function extraction. A function that does one thing in one place is easier to
  follow than three helpers scattered across the file.
- **No dead code, no unused imports.** Clippy runs with `--deny warnings` in CI.
- **GTK thread safety:** GTK widgets are `!Send`. All widget access must happen
  on the main thread. Cross-thread communication uses `std::sync::mpsc` channels
  drained by `glib::timeout_add_local` timers.

## GTK4 / glib 0.20 notes

- `CssProvider::load_from_data(&str)` — takes `&str`, not `&[u8]`.
- `glib::MainContext::channel` does not exist in 0.20. Use `mpsc` + timer.
- `gtk4_layer_shell` 0.5: `window.set_namespace(Some("name"))` sets the
  Hyprland layerrule target name.
- `LD_LIBRARY_PATH` must be set for `cargo run` in the devShell; `wrapProgram`
  handles it for `nix run`.

## IPC protocol

Plain text over a Unix socket. Each message is a single line.

| Client sends | Daemon responds  |
|--------------|------------------|
| `show\n`     | `OK shown\n`     |
| `hide\n`     | `OK hidden\n`    |
| `toggle\n`   | `OK toggled\n`   |
| `quit\n`     | `OK quitting\n`  |
| unknown      | `ERR unknown: …` |

## Claude usage API

- **Endpoint:** `GET https://api.anthropic.com/api/oauth/usage`
- **Auth:** `Authorization: Bearer <token>` from `~/.claude/.credentials.json`
- **Header:** `anthropic-beta: oauth-2025-04-20`
- **Key fields:** `five_hour.utilization`, `seven_day.utilization`,
  `extra_usage.used_credits`, `extra_usage.monthly_limit` (cents)
- **Local stats:** `~/.claude/stats-cache.json` → `dailyActivity[].messageCount`
- Refreshed every 60 s from a background thread.
