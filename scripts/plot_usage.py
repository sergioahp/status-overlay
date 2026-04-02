#!/usr/bin/env python3

import argparse
import json
import os
import sqlite3
from datetime import datetime
from pathlib import Path

import matplotlib.pyplot as plt
import pandas as pd


DEFAULT_STATE_HOME = Path(os.environ["XDG_STATE_HOME"]) if "XDG_STATE_HOME" in os.environ else Path.home() / ".local" / "state"
DEFAULT_STATUS_OVERLAY_STATE = DEFAULT_STATE_HOME / "status-overlay"
DEFAULT_CLAUDE_STATS = Path.home() / ".claude" / "stats-cache.json"
DEFAULT_CLAUDE_HISTORY = Path.home() / ".claude" / "history.jsonl"
DEFAULT_HISTORY_DB = DEFAULT_STATUS_OVERLAY_STATE / "history.sqlite3"
DEFAULT_CODEX_HISTORY = DEFAULT_STATUS_OVERLAY_STATE / "codex_history.json"
DEFAULT_OUTPUT_DIR = Path.cwd() / "plots" / "usage"
LOCAL_TZ = datetime.now().astimezone().tzinfo
SESSION_GAP_CANDIDATES_MINUTES = [15, 20, 30, 45, 60, 90, 120, 180]
MIN_SESSION_MINUTES_FOR_BURN = 5.0


def load_json(path: Path):
    with path.open() as fh:
        return json.load(fh)


def load_claude_stats(path: Path) -> dict:
    return load_json(path)


def load_claude_activity(path: Path) -> pd.DataFrame:
    raw = load_claude_stats(path)
    daily = pd.DataFrame(raw.get("dailyActivity", []))
    if daily.empty:
        return daily
    daily["date"] = pd.to_datetime(daily["date"])
    daily = daily.sort_values("date").reset_index(drop=True)
    daily["tool_calls_per_100_messages"] = daily["toolCallCount"] / daily["messageCount"] * 100.0
    daily["messages_7d_avg"] = daily["messageCount"].rolling(window=7, min_periods=1).mean()
    daily["tool_calls_7d_avg"] = daily["toolCallCount"].rolling(window=7, min_periods=1).mean()
    return daily


def load_claude_model_tokens(path: Path) -> pd.DataFrame:
    raw = load_claude_stats(path)
    rows = []
    for entry in raw.get("dailyModelTokens", []):
        day = entry.get("date")
        for model, tokens in entry.get("tokensByModel", {}).items():
            rows.append({"date": day, "model": model, "tokens": tokens})
    frame = pd.DataFrame(rows)
    if frame.empty:
        return frame
    frame["date"] = pd.to_datetime(frame["date"])
    frame = frame.sort_values(["date", "model"]).reset_index(drop=True)
    return frame


def load_claude_history_timestamps(path: Path) -> pd.DataFrame:
    rows = []
    with path.open() as fh:
        for line in fh:
            try:
                obj = json.loads(line)
            except json.JSONDecodeError:
                continue
            ts = obj.get("timestamp") if isinstance(obj, dict) else None
            if not isinstance(ts, int):
                continue
            dt = datetime.fromtimestamp(ts / 1000, tz=LOCAL_TZ).replace(tzinfo=None)
            rows.append({"datetime": dt})
    frame = pd.DataFrame(rows)
    if frame.empty:
        return frame
    return frame.sort_values("datetime").reset_index(drop=True)


def load_claude_quota_history(db_path: Path) -> pd.DataFrame:
    with sqlite3.connect(db_path) as conn:
        frame = pd.read_sql_query(
            """
            SELECT fetched_at AS ts, session_pct, weekly_pct, plan
            FROM claude_usage_samples
            ORDER BY fetched_at, id
            """,
            conn,
        )
    if frame.empty:
        return frame
    frame["datetime"] = pd.to_datetime(frame["ts"], unit="s")
    frame = frame.drop_duplicates(subset=["datetime", "session_pct", "weekly_pct"]).sort_values("datetime").reset_index(drop=True)
    frame["session_delta"] = frame["session_pct"].diff().fillna(0)
    frame["weekly_delta"] = frame["weekly_pct"].diff().fillna(0)
    return frame


def load_codex_history_sqlite(path: Path) -> pd.DataFrame:
    with sqlite3.connect(path) as conn:
        frame = pd.read_sql_query(
            """
            SELECT fetched_at AS ts, primary_pct, secondary_pct
            FROM codex_usage_samples
            ORDER BY fetched_at, id
            """,
            conn,
        )
    if frame.empty:
        return frame
    frame["datetime"] = pd.to_datetime(frame["ts"], unit="s")
    frame = frame.sort_values("datetime").reset_index(drop=True)
    frame = frame.drop_duplicates(subset=["datetime", "primary_pct", "secondary_pct"])
    frame["minutes_from_start"] = (
        frame["datetime"] - frame["datetime"].iloc[0]
    ).dt.total_seconds() / 60.0
    return frame


def load_codex_history_json(path: Path) -> pd.DataFrame:
    raw = load_json(path)
    frame = pd.DataFrame(raw)
    if frame.empty:
        return frame
    frame["datetime"] = pd.to_datetime(frame["ts"], unit="s")
    frame = frame.sort_values("datetime").reset_index(drop=True)
    frame = frame.drop_duplicates(subset=["datetime", "primary_pct", "secondary_pct"])
    frame["minutes_from_start"] = (
        frame["datetime"] - frame["datetime"].iloc[0]
    ).dt.total_seconds() / 60.0
    return frame


def choose_quota_zoom_windows(history: pd.DataFrame) -> list[tuple[str, pd.Timestamp, pd.Timestamp]]:
    if history.empty:
        return []
    reset_points = history.loc[history["session_delta"] < -20, "datetime"].tolist()
    windows = []
    for index, point in enumerate(reset_points[-2:], start=1):
        windows.append((f"reset_{index}", point - pd.Timedelta(hours=6), point + pd.Timedelta(hours=6)))
    if windows:
        return windows
    end = history["datetime"].max()
    start = max(history["datetime"].min(), end - pd.Timedelta(hours=24))
    return [("last_24h", start, end)]


def configure_axes(ax, title: str, ylabel: str):
    ax.set_title(title)
    ax.set_ylabel(ylabel)
    ax.grid(alpha=0.25)


def build_claude_daily_frame(activity: pd.DataFrame, model_tokens: pd.DataFrame) -> pd.DataFrame:
    if activity.empty:
        return pd.DataFrame()
    token_totals = (
        model_tokens.groupby("date", as_index=False)["tokens"]
        .sum()
        .rename(columns={"tokens": "total_tokens"})
        if not model_tokens.empty
        else pd.DataFrame(columns=["date", "total_tokens"])
    )
    daily = activity.merge(token_totals, on="date", how="left")
    daily["total_tokens"] = daily["total_tokens"].fillna(0)
    daily["messages_per_session"] = daily["messageCount"] / daily["sessionCount"].clip(lower=1)
    daily["tool_calls_per_session"] = daily["toolCallCount"] / daily["sessionCount"].clip(lower=1)
    daily["tokens_per_session"] = daily["total_tokens"] / daily["sessionCount"].clip(lower=1)
    return daily


def infer_sessions(timestamps: pd.DataFrame, gap_minutes: int) -> pd.DataFrame:
    if timestamps.empty:
        return pd.DataFrame()
    gap = pd.Timedelta(minutes=gap_minutes)
    sessions = []
    start = timestamps["datetime"].iloc[0]
    end = start
    count = 1
    for current in timestamps["datetime"].iloc[1:]:
        if current - end > gap:
            sessions.append({"start": start, "end": end, "record_count": count})
            start = current
            end = current
            count = 1
        else:
            end = current
            count += 1
    sessions.append({"start": start, "end": end, "record_count": count})
    frame = pd.DataFrame(sessions)
    frame["date"] = pd.to_datetime(frame["start"]).dt.normalize()
    frame["duration_minutes"] = (frame["end"] - frame["start"]).dt.total_seconds() / 60.0
    frame["effective_duration_minutes"] = frame["duration_minutes"].clip(lower=MIN_SESSION_MINUTES_FOR_BURN)
    frame["duration_hours"] = frame["duration_minutes"] / 60.0
    frame["effective_duration_hours"] = frame["effective_duration_minutes"] / 60.0
    return frame


def choose_session_gap_minutes(activity: pd.DataFrame, timestamps: pd.DataFrame) -> tuple[int, float]:
    if activity.empty or timestamps.empty:
        return 60, float("nan")
    expected = activity.set_index(activity["date"].dt.date)["sessionCount"].to_dict()
    best_gap = SESSION_GAP_CANDIDATES_MINUTES[0]
    best_mae = float("inf")
    for minutes in SESSION_GAP_CANDIDATES_MINUTES:
        sessions = infer_sessions(timestamps, minutes)
        actual = sessions["date"].dt.date.value_counts().to_dict()
        common_days = sorted(set(expected) & set(actual))
        if not common_days:
            continue
        mae = sum(abs(expected[day] - actual.get(day, 0)) for day in common_days) / len(common_days)
        if mae < best_mae:
            best_gap = minutes
            best_mae = mae
    return best_gap, best_mae


def build_inferred_daily_frame(daily: pd.DataFrame, sessions: pd.DataFrame) -> pd.DataFrame:
    if daily.empty or sessions.empty:
        return daily
    inferred = (
        sessions.groupby("date", as_index=False)
        .agg(
            inferred_session_count=("start", "count"),
            inferred_active_hours=("effective_duration_hours", "sum"),
            median_session_minutes=("duration_minutes", "median"),
            max_session_hours=("duration_hours", "max"),
            session_records=("record_count", "sum"),
        )
    )
    merged = daily.merge(inferred, on="date", how="left")
    merged["inferred_active_hours"] = merged["inferred_active_hours"].fillna(0)
    merged["inferred_session_count"] = merged["inferred_session_count"].fillna(0)
    merged["tokens_per_active_hour"] = merged["total_tokens"] / merged["inferred_active_hours"].replace(0, pd.NA)
    return merged


def build_session_token_estimates(sessions: pd.DataFrame, daily: pd.DataFrame) -> pd.DataFrame:
    if sessions.empty or daily.empty:
        return pd.DataFrame()
    frame = sessions.merge(daily[["date", "total_tokens"]], on="date", how="inner")
    if frame.empty:
        return frame
    frame["day_record_total"] = frame.groupby("date")["record_count"].transform("sum")
    frame["estimated_tokens"] = frame["total_tokens"] * frame["record_count"] / frame["day_record_total"].clip(lower=1)
    frame["estimated_tokens_per_hour"] = frame["estimated_tokens"] / frame["effective_duration_hours"].clip(lower=MIN_SESSION_MINUTES_FOR_BURN / 60.0)
    return frame


def plot_claude_daily_volume(activity: pd.DataFrame, out_dir: Path):
    fig, ax = plt.subplots(figsize=(11, 5))
    ax.bar(activity["date"], activity["messageCount"], width=0.8, color="#1f77b4", alpha=0.8, label="Messages")
    ax.plot(activity["date"], activity["messages_7d_avg"], color="#0d3b66", linewidth=2, label="7-day avg")
    configure_axes(ax, "Claude Daily Message Volume", "Messages")
    ax.set_xlabel("Date")
    ax.legend()
    fig.autofmt_xdate()
    fig.tight_layout()
    fig.savefig(out_dir / "claude_daily_messages.png", dpi=150)
    plt.close(fig)


def plot_claude_tool_intensity(activity: pd.DataFrame, out_dir: Path):
    fig, ax = plt.subplots(figsize=(11, 5))
    ax.bar(activity["date"], activity["toolCallCount"], width=0.8, color="#2a9d8f", alpha=0.8, label="Tool calls")
    ax.plot(activity["date"], activity["tool_calls_7d_avg"], color="#1d6f63", linewidth=2, label="7-day avg")
    ax2 = ax.twinx()
    ax2.plot(
        activity["date"],
        activity["tool_calls_per_100_messages"],
        color="#e76f51",
        marker="o",
        linewidth=1.5,
        label="Tool calls per 100 messages",
    )
    configure_axes(ax, "Claude Tool Usage Intensity", "Tool calls")
    ax.set_xlabel("Date")
    ax2.set_ylabel("Calls / 100 messages")
    handles1, labels1 = ax.get_legend_handles_labels()
    handles2, labels2 = ax2.get_legend_handles_labels()
    ax.legend(handles1 + handles2, labels1 + labels2, loc="upper left")
    fig.autofmt_xdate()
    fig.tight_layout()
    fig.savefig(out_dir / "claude_tool_intensity.png", dpi=150)
    plt.close(fig)


def plot_claude_model_mix(model_tokens: pd.DataFrame, out_dir: Path):
    pivot = model_tokens.pivot_table(index="date", columns="model", values="tokens", aggfunc="sum", fill_value=0)
    fig, ax = plt.subplots(figsize=(11, 5))
    pivot.plot(kind="bar", stacked=True, ax=ax, width=0.85, colormap="tab20")
    configure_axes(ax, "Claude Tokens by Model and Day", "Tokens")
    ax.set_xlabel("Date")
    ax.legend(title="Model", fontsize=8)
    fig.tight_layout()
    fig.savefig(out_dir / "claude_model_tokens.png", dpi=150)
    plt.close(fig)


def plot_claude_quota_history(history: pd.DataFrame, out_dir: Path):
    if history.empty:
        return
    fig, ax = plt.subplots(figsize=(11, 5))
    ax.step(history["datetime"], history["session_pct"], where="post", color="#b22222", linewidth=2, label="5h session % used")
    ax.step(history["datetime"], history["weekly_pct"], where="post", color="#1d3557", linewidth=2, label="7d weekly % used")
    configure_axes(ax, "Claude Quota Usage History", "Percent used")
    ax.set_xlabel("Time")
    ax.set_ylim(0, 100)
    ax.legend()
    fig.autofmt_xdate()
    fig.tight_layout()
    fig.savefig(out_dir / "claude_quota_history.png", dpi=150)
    plt.close(fig)


def plot_claude_quota_zoom(history: pd.DataFrame, out_dir: Path):
    if history.empty:
        return
    for label, start, end in choose_quota_zoom_windows(history):
        window = history[(history["datetime"] >= start) & (history["datetime"] <= end)].copy()
        if len(window) < 2:
            continue
        fig, ax = plt.subplots(figsize=(11, 5))
        ax.step(window["datetime"], window["session_pct"], where="post", color="#b22222", linewidth=2, label="5h session % used")
        ax.step(window["datetime"], window["weekly_pct"], where="post", color="#1d3557", linewidth=2, label="7d weekly % used")
        configure_axes(ax, f"Claude Quota Zoom: {label.replace('_', ' ')}", "Percent used")
        ax.set_xlabel("Time")
        ax.set_ylim(0, 100)
        ax.legend()
        fig.autofmt_xdate()
        fig.tight_layout()
        fig.savefig(out_dir / f"claude_quota_zoom_{label}.png", dpi=150)
        plt.close(fig)


def plot_claude_tokens_per_session(daily: pd.DataFrame, out_dir: Path):
    fig, ax = plt.subplots(figsize=(11, 5))
    ax.bar(daily["date"], daily["tokens_per_session"], width=0.8, color="#6d597a", alpha=0.85, label="Estimated tokens / official session")
    ax.set_xlabel("Date")
    configure_axes(ax, "Claude Tokens Per Session", "Tokens / session")
    ax2 = ax.twinx()
    ax2.plot(
        daily["date"],
        daily["messages_per_session"],
        color="#e56b6f",
        marker="o",
        linewidth=1.5,
        label="Messages / session",
    )
    ax2.set_ylabel("Messages / session")
    handles1, labels1 = ax.get_legend_handles_labels()
    handles2, labels2 = ax2.get_legend_handles_labels()
    ax.legend(handles1 + handles2, labels1 + labels2, loc="upper left")
    fig.autofmt_xdate()
    fig.tight_layout()
    fig.savefig(out_dir / "claude_tokens_per_session.png", dpi=150)
    plt.close(fig)


def plot_claude_active_hours(daily: pd.DataFrame, gap_minutes: int, out_dir: Path):
    fig, ax = plt.subplots(figsize=(11, 5))
    ax.bar(daily["date"], daily["inferred_active_hours"], width=0.8, color="#588157", alpha=0.85, label="Inferred active hours")
    configure_axes(ax, f"Claude Active Time by Day ({gap_minutes} min inactivity split)", "Active hours")
    ax.set_xlabel("Date")
    ax2 = ax.twinx()
    ax2.plot(
        daily["date"],
        daily["sessionCount"],
        color="#344e41",
        marker="o",
        linewidth=1.5,
        label="Official sessions / day",
    )
    ax2.plot(
        daily["date"],
        daily["inferred_session_count"],
        color="#dda15e",
        marker="s",
        linewidth=1.5,
        label="Inferred sessions / day",
    )
    ax2.set_ylabel("Sessions")
    handles1, labels1 = ax.get_legend_handles_labels()
    handles2, labels2 = ax2.get_legend_handles_labels()
    ax.legend(handles1 + handles2, labels1 + labels2, loc="upper left")
    fig.autofmt_xdate()
    fig.tight_layout()
    fig.savefig(out_dir / "claude_active_hours.png", dpi=150)
    plt.close(fig)


def plot_claude_tokens_per_active_hour(daily: pd.DataFrame, gap_minutes: int, out_dir: Path):
    frame = daily.dropna(subset=["tokens_per_active_hour"]).copy()
    if frame.empty:
        return
    frame["tokens_per_active_hour_7d_avg"] = frame["tokens_per_active_hour"].rolling(window=7, min_periods=1).mean()
    fig, ax = plt.subplots(figsize=(11, 5))
    ax.bar(frame["date"], frame["tokens_per_active_hour"], width=0.8, color="#bc4749", alpha=0.85, label="Estimated tokens / active hour")
    ax.plot(frame["date"], frame["tokens_per_active_hour_7d_avg"], color="#6a040f", linewidth=2, label="7-day avg")
    configure_axes(ax, f"Claude Burn Rate Proxy ({gap_minutes} min inactivity split)", "Estimated tokens / active hour")
    ax.set_xlabel("Date")
    ax.legend()
    fig.autofmt_xdate()
    fig.tight_layout()
    fig.savefig(out_dir / "claude_tokens_per_active_hour.png", dpi=150)
    plt.close(fig)


def plot_claude_inferred_session_durations(sessions: pd.DataFrame, gap_minutes: int, out_dir: Path):
    if sessions.empty:
        return
    fig, ax = plt.subplots(figsize=(11, 5))
    ax.scatter(sessions["start"], sessions["duration_hours"], color="#277da1", alpha=0.7, s=22)
    ax.axhline(5.0, color="#d62828", linestyle="--", linewidth=1.5, label="5h window")
    configure_axes(ax, f"Claude Inferred Session Durations ({gap_minutes} min inactivity split)", "Duration (hours)")
    ax.set_xlabel("Session start")
    ax.legend()
    fig.autofmt_xdate()
    fig.tight_layout()
    fig.savefig(out_dir / "claude_inferred_session_durations.png", dpi=150)
    plt.close(fig)


def plot_claude_session_burn_proxy(session_estimates: pd.DataFrame, out_dir: Path):
    if session_estimates.empty:
        return
    fig, ax = plt.subplots(figsize=(11, 5))
    scatter = ax.scatter(
        session_estimates["duration_hours"].clip(lower=MIN_SESSION_MINUTES_FOR_BURN / 60.0),
        session_estimates["estimated_tokens_per_hour"],
        c=session_estimates["estimated_tokens"],
        cmap="viridis",
        alpha=0.75,
        s=24 + session_estimates["record_count"].clip(upper=40) * 2,
    )
    configure_axes(ax, "Claude Session Burn Proxy", "Estimated tokens / active hour")
    ax.set_xlabel("Inferred session duration (hours)")
    ax.set_yscale("log")
    cbar = fig.colorbar(scatter, ax=ax)
    cbar.set_label("Estimated session tokens")
    fig.tight_layout()
    fig.savefig(out_dir / "claude_session_burn_proxy.png", dpi=150)
    plt.close(fig)


def plot_codex_session_burn(history: pd.DataFrame, out_dir: Path):
    fig, ax = plt.subplots(figsize=(11, 5))
    ax.step(history["datetime"], history["primary_pct"], where="post", color="#264653", linewidth=2, label="Primary window")
    ax.step(history["datetime"], history["secondary_pct"], where="post", color="#e9c46a", linewidth=2, label="Secondary window")
    configure_axes(ax, "Codex Session Burn-Up", "Used percent")
    ax.set_xlabel("Time")
    ax.set_ylim(0, 100)
    ax.legend()
    fig.autofmt_xdate()
    fig.tight_layout()
    fig.savefig(out_dir / "codex_session_burn.png", dpi=150)
    plt.close(fig)


def build_summary(
    stats_meta: dict,
    activity: pd.DataFrame,
    daily: pd.DataFrame,
    model_tokens: pd.DataFrame,
    claude_quota_history: pd.DataFrame,
    claude_sessions: pd.DataFrame,
    session_gap_minutes: int | None,
    session_gap_mae: float | None,
    session_estimates: pd.DataFrame,
    history: pd.DataFrame,
) -> str:
    lines = []
    if not activity.empty:
        top_messages = activity.loc[activity["messageCount"].idxmax()]
        top_tools = activity.loc[activity["toolCallCount"].idxmax()]
        busiest_ratio = activity.loc[activity["tool_calls_per_100_messages"].idxmax()]
        lines.append(
            f"Claude daily history: {len(activity)} days from {activity['date'].min().date()} to {activity['date'].max().date()}."
        )
        lines.append(
            f"Peak message day: {top_messages['date'].date()} with {int(top_messages['messageCount'])} messages."
        )
        lines.append(
            f"Peak tool-call day: {top_tools['date'].date()} with {int(top_tools['toolCallCount'])} tool calls."
        )
        lines.append(
            f"Highest tool intensity: {busiest_ratio['date'].date()} at {busiest_ratio['tool_calls_per_100_messages']:.1f} tool calls per 100 messages."
        )
        if stats_meta.get("lastComputedDate"):
            lines.append(f"Claude stats-cache last computed date: {stats_meta['lastComputedDate']}.")
    if not daily.empty:
        top_token_session = daily.loc[daily["tokens_per_session"].idxmax()]
        lines.append(
            f"Highest estimated Claude tokens per session: {top_token_session['date'].date()} at {top_token_session['tokens_per_session']:.0f} tokens/session."
        )
        active_days = daily.dropna(subset=["tokens_per_active_hour"])
        if not active_days.empty:
            top_burn = active_days.loc[active_days["tokens_per_active_hour"].idxmax()]
            lines.append(
                f"Fastest burn-rate day proxy: {top_burn['date'].date()} at {top_burn['tokens_per_active_hour']:.0f} estimated tokens per active hour."
            )
    if not model_tokens.empty:
        totals = model_tokens.groupby("model")["tokens"].sum().sort_values(ascending=False)
        leader = totals.index[0]
        share = totals.iloc[0] / totals.sum() * 100.0
        lines.append(f"Top Claude model by tokens: {leader} at {share:.1f}% of tracked daily tokens.")
    if not claude_quota_history.empty:
        reset_count = int((claude_quota_history["session_delta"] < -20).sum())
        lines.append(
            f"Claude quota history: {len(claude_quota_history)} samples from {claude_quota_history['datetime'].min()} to {claude_quota_history['datetime'].max()}."
        )
        lines.append(
            f"Observed {reset_count} large session reset drops in the recorded quota history."
        )
    else:
        lines.append("Claude quota history: no sqlite samples recorded yet.")
    if not claude_sessions.empty and session_gap_minutes is not None:
        longest = claude_sessions.loc[claude_sessions["duration_hours"].idxmax()]
        lines.append(
            f"Inferred Claude sessions: {len(claude_sessions)} using a {session_gap_minutes}-minute inactivity split (daily session-count MAE {session_gap_mae:.2f})."
        )
        lines.append(
            f"Longest inferred session span: {longest['start'].date()} at {longest['duration_hours']:.2f} hours."
        )
    if not session_estimates.empty:
        top_session = session_estimates.loc[session_estimates["estimated_tokens_per_hour"].idxmax()]
        lines.append(
            f"Fastest inferred session burn proxy: {top_session['start'].date()} at {top_session['estimated_tokens_per_hour']:.0f} estimated tokens/hour."
        )
    if not history.empty:
        start = history.iloc[0]
        end = history.iloc[-1]
        duration_minutes = history["minutes_from_start"].iloc[-1]
        lines.append(
            f"Codex history: {len(history)} unique points from {start['datetime']} to {end['datetime']} ({duration_minutes:.1f} minutes)."
        )
        lines.append(
            f"Codex primary moved from {int(start['primary_pct'])}% to {int(end['primary_pct'])}%; secondary moved from {int(start['secondary_pct'])}% to {int(end['secondary_pct'])}%."
        )
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description="Generate usage plots from local Claude and Codex history.")
    parser.add_argument("--claude-stats", type=Path, default=DEFAULT_CLAUDE_STATS)
    parser.add_argument("--claude-history", type=Path, default=DEFAULT_CLAUDE_HISTORY)
    parser.add_argument("--history-db", type=Path, default=DEFAULT_HISTORY_DB)
    parser.add_argument("--codex-history", type=Path, default=DEFAULT_CODEX_HISTORY)
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_OUTPUT_DIR)
    args = parser.parse_args()

    args.out_dir.mkdir(parents=True, exist_ok=True)

    stats_meta = {}
    activity = pd.DataFrame()
    daily = pd.DataFrame()
    model_tokens = pd.DataFrame()
    claude_quota_history = pd.DataFrame()
    claude_timestamps = pd.DataFrame()
    claude_sessions = pd.DataFrame()
    session_estimates = pd.DataFrame()
    session_gap_minutes = None
    session_gap_mae = None
    history = pd.DataFrame()

    if args.claude_stats.exists():
        stats_meta = load_claude_stats(args.claude_stats)
        activity = load_claude_activity(args.claude_stats)
        model_tokens = load_claude_model_tokens(args.claude_stats)
        daily = build_claude_daily_frame(activity, model_tokens)
    if args.history_db.exists():
        claude_quota_history = load_claude_quota_history(args.history_db)
        history = load_codex_history_sqlite(args.history_db)
    if args.claude_history.exists():
        claude_timestamps = load_claude_history_timestamps(args.claude_history)
    if not activity.empty and not claude_timestamps.empty:
        session_gap_minutes, session_gap_mae = choose_session_gap_minutes(activity, claude_timestamps)
        claude_sessions = infer_sessions(claude_timestamps, session_gap_minutes)
        daily = build_inferred_daily_frame(daily, claude_sessions)
        session_estimates = build_session_token_estimates(claude_sessions, daily)
    if history.empty and args.codex_history.exists():
        history = load_codex_history_json(args.codex_history)

    if activity.empty and model_tokens.empty and claude_quota_history.empty and claude_sessions.empty and history.empty:
        raise SystemExit("No input data found.")

    if not activity.empty:
        plot_claude_daily_volume(activity, args.out_dir)
        plot_claude_tool_intensity(activity, args.out_dir)
    if not claude_quota_history.empty:
        plot_claude_quota_history(claude_quota_history, args.out_dir)
        plot_claude_quota_zoom(claude_quota_history, args.out_dir)
    if not daily.empty:
        plot_claude_tokens_per_session(daily, args.out_dir)
    if not daily.empty and "inferred_active_hours" in daily:
        plot_claude_active_hours(daily, session_gap_minutes, args.out_dir)
        plot_claude_tokens_per_active_hour(daily, session_gap_minutes, args.out_dir)
    if not model_tokens.empty:
        plot_claude_model_mix(model_tokens, args.out_dir)
    if not claude_sessions.empty and session_gap_minutes is not None:
        plot_claude_inferred_session_durations(claude_sessions, session_gap_minutes, args.out_dir)
    if not session_estimates.empty:
        plot_claude_session_burn_proxy(session_estimates, args.out_dir)
    if not history.empty:
        plot_codex_session_burn(history, args.out_dir)

    summary = build_summary(
        stats_meta,
        activity,
        daily,
        model_tokens,
        claude_quota_history,
        claude_sessions,
        session_gap_minutes,
        session_gap_mae,
        session_estimates,
        history,
    )
    (args.out_dir / "summary.txt").write_text(summary)
    print(summary, end="")
    print(f"Plots written to {args.out_dir}")


if __name__ == "__main__":
    main()
