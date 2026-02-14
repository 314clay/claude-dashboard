#!/usr/bin/env python3
"""
Ingest Claude Code local history into the dashboard SQLite database.

Reads session transcripts from ~/.claude/projects/ and stats from
~/.claude/stats-cache.json, then inserts into the SQLite database
used by dashboard-native.

Usage:
    python ingest.py                          # Import all sessions
    python ingest.py --since 7d               # Last 7 days only
    python ingest.py --path /other/.claude/   # Custom Claude dir
    python ingest.py --db /path/to/db.sqlite  # Custom database path
    python ingest.py --stats-only             # Only import stats-cache.json
"""

import argparse
import json
import os
import re
import sqlite3
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path


DEFAULT_CLAUDE_DIR = Path.home() / ".claude"
DEFAULT_DB_PATH = (
    Path(os.environ.get("DB_PATH", ""))
    if os.environ.get("DB_PATH")
    else Path.home() / ".config" / "dashboard-native" / "dashboard.db"
)
SCHEMA_FILE = Path(__file__).parent / "schema.sqlite.sql"


AGENT_PATH_KEYWORDS = {"polecats", "crew", "witness", "refinery", "mayor"}


def detect_agent_type(project_path: str) -> str | None:
    """Detect if a session path indicates an agent session.

    Returns the agent type string (e.g. 'polecat', 'witness') or None for human sessions.
    """
    if not project_path:
        return None
    parts = project_path.lower().replace("\\", "/").split("/")
    for part in parts:
        if part == "polecats":
            return "polecat"
        if part in AGENT_PATH_KEYWORDS:
            return part
    return None


def parse_since(since_str: str) -> datetime:
    """Parse a --since string like '7d', '24h', '30d' into a cutoff datetime."""
    match = re.match(r"^(\d+)([dhm])$", since_str)
    if not match:
        raise ValueError(f"Invalid --since format: {since_str!r}. Use e.g. '7d', '24h', '30m'")
    value, unit = int(match.group(1)), match.group(2)
    delta = {"d": timedelta(days=value), "h": timedelta(hours=value), "m": timedelta(minutes=value)}[unit]
    return datetime.now(timezone.utc) - delta


def init_db(db_path: Path) -> sqlite3.Connection:
    """Create database and apply schema if needed."""
    db_path.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(str(db_path))
    conn.execute("PRAGMA journal_mode = WAL")
    conn.execute("PRAGMA busy_timeout = 30000")
    conn.execute("PRAGMA foreign_keys = ON")

    if SCHEMA_FILE.exists():
        schema = SCHEMA_FILE.read_text()
        conn.executescript(schema)
    else:
        print(f"Warning: schema file not found at {SCHEMA_FILE}", file=sys.stderr)
        print("Database tables must already exist.", file=sys.stderr)

    return conn


def discover_sessions(claude_dir: Path, since: datetime | None = None) -> list[dict]:
    """Find all sessions from sessions-index.json files and unindexed JSONL files."""
    projects_dir = claude_dir / "projects"
    if not projects_dir.exists():
        print(f"No projects directory found at {projects_dir}", file=sys.stderr)
        return []

    sessions = []
    indexed_ids: set[str] = set()

    for project_dir in projects_dir.iterdir():
        if not project_dir.is_dir():
            continue

        index_path = project_dir / "sessions-index.json"
        original_path = ""

        # Phase 1: Read the sessions-index.json if it exists
        if index_path.exists():
            try:
                with open(index_path) as f:
                    data = json.load(f)
            except (json.JSONDecodeError, OSError) as e:
                print(f"  Skipping {index_path}: {e}", file=sys.stderr)
                data = {}

            entries = data.get("entries", []) if isinstance(data, dict) else data
            original_path = data.get("originalPath", "") if isinstance(data, dict) else ""

        # Decode directory name to real path when no index provides one.
        # Dir names use '-' as separator, but real dirs may contain hyphens
        # (e.g. "dashboard-native"). Greedily resolve segments left-to-right.
        if not original_path:
            parts = project_dir.name.lstrip("-").split("-")
            built = "/"
            i = 0
            while i < len(parts):
                # Try joining progressively more segments with hyphens
                for j in range(len(parts), i, -1):
                    candidate = built.rstrip("/") + "/" + "-".join(parts[i:j])
                    if Path(candidate).exists():
                        built = candidate
                        i = j
                        break
                else:
                    # No combination matched; look for Gas Town structural keywords
                    rest = parts[i:]
                    gt_keywords = {"polecats", "crew", "witness", "refinery"}
                    # Check if we already resolved onto a keyword (e.g. .../polecats)
                    parent_seg = Path(built).name
                    if parent_seg in gt_keywords and rest:
                        if parent_seg == "polecats" and len(rest) >= 2:
                            built = built + "/" + rest[0] + "/" + "-".join(rest[1:])
                        else:
                            built = built + "/" + "-".join(rest)
                    else:
                        split_idx = next((k for k, p in enumerate(rest) if p in gt_keywords), None)
                        if split_idx is not None:
                            if split_idx > 0:
                                built = built.rstrip("/") + "/" + "-".join(rest[:split_idx])
                            remaining = rest[split_idx:]
                            kw = remaining[0]
                            built = built.rstrip("/") + "/" + kw
                            after = remaining[1:]
                            if kw == "polecats" and len(after) >= 2:
                                built = built + "/" + after[0] + "/" + "-".join(after[1:])
                            elif after:
                                built = built + "/" + "-".join(after)
                        else:
                            built = built.rstrip("/") + "/" + "-".join(rest)
                    break
            if built != "/":
                original_path = built

        if index_path.exists():
            for entry in entries:
                created = entry.get("created", "")
                if since and created:
                    try:
                        ts = datetime.fromisoformat(created.replace("Z", "+00:00"))
                        if ts < since:
                            continue
                    except ValueError:
                        pass

                sid = entry.get("sessionId", "")
                if sid:
                    indexed_ids.add(sid)
                entry["_project_path"] = entry.get("projectPath", original_path)
                entry["_project_dir"] = str(project_dir)
                sessions.append(entry)

        # Phase 2: Discover JSONL files not present in the index
        since_ts = since.timestamp() if since else None
        for jsonl_path in project_dir.glob("*.jsonl"):
            session_id = jsonl_path.stem
            if session_id in indexed_ids:
                continue

            # Apply --since filter using file mtime
            if since_ts and jsonl_path.stat().st_mtime < since_ts:
                continue

            # Build a synthetic index entry from the JSONL file
            mtime = datetime.fromtimestamp(jsonl_path.stat().st_mtime, tz=timezone.utc)
            sessions.append({
                "sessionId": session_id,
                "created": mtime.isoformat(),
                "modified": mtime.isoformat(),
                "_project_path": original_path,
                "_project_dir": str(project_dir),
                "fullPath": str(jsonl_path),
            })

    return sessions


def parse_transcript(jsonl_path: Path, agent_type: str | None = None) -> dict:
    """Parse a session JSONL transcript into messages and tool usages.

    Args:
        jsonl_path: Path to the JSONL transcript file.
        agent_type: If set, remap "user" roles to this agent type
                    (e.g. "polecat", "witness") so they are distinguishable
                    from human user messages.

    Returns:
        {
            "messages": [{"role", "content", "timestamp", "model", "usage", "sequence_num"}],
            "tool_usages": [{"tool_name", "tool_input", "message_index"}],
        }
    """
    messages = []
    tool_usages = []
    msg_seq = 0

    if not jsonl_path.exists():
        return {"messages": [], "tool_usages": []}

    with open(jsonl_path) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                entry = json.loads(line)
            except json.JSONDecodeError:
                continue

            entry_type = entry.get("type", "")
            if entry_type not in ("user", "assistant"):
                continue

            msg_data = entry.get("message", {})
            role = msg_data.get("role", entry_type)

            # Remap "user" role to agent type for agent sessions
            if agent_type and role == "user":
                role = agent_type

            timestamp = entry.get("timestamp", "")
            content_raw = msg_data.get("content", "")

            # Extract text content and tool uses from content blocks
            text_parts = []
            msg_tools = []

            if isinstance(content_raw, str):
                text_parts.append(content_raw)
            elif isinstance(content_raw, list):
                for block in content_raw:
                    if not isinstance(block, dict):
                        continue
                    block_type = block.get("type", "")
                    if block_type == "text":
                        text_parts.append(block.get("text", ""))
                    elif block_type == "tool_use":
                        msg_tools.append({
                            "tool_name": block.get("name", ""),
                            "tool_input": json.dumps(block.get("input", {})),
                            "message_index": msg_seq,
                        })
                    # Skip thinking, tool_result, etc.

            content = "\n".join(text_parts).strip()

            # Skip empty messages (e.g. pure thinking blocks)
            if not content and not msg_tools:
                continue

            # For assistant messages that only have tool calls, create a summary
            if not content and msg_tools:
                tool_names = [t["tool_name"] for t in msg_tools]
                content = f"[Used tools: {', '.join(tool_names)}]"

            # Extract token usage
            usage = msg_data.get("usage", {})
            model = msg_data.get("model", "")

            messages.append({
                "role": role,
                "content": content,
                "timestamp": timestamp,
                "model": model,
                "input_tokens": usage.get("input_tokens"),
                "output_tokens": usage.get("output_tokens"),
                "cache_read_tokens": usage.get("cache_read_input_tokens"),
                "cache_creation_tokens": usage.get("cache_creation_input_tokens"),
                "sequence_num": msg_seq,
            })

            tool_usages.extend(msg_tools)
            msg_seq += 1

    return {"messages": messages, "tool_usages": tool_usages}


def import_session(conn: sqlite3.Connection, session_info: dict, claude_dir: Path) -> dict:
    """Import a single session into the database.

    Returns stats dict: {"messages": N, "tools": N, "skipped": bool}
    """
    session_id = session_info.get("sessionId", "")
    if not session_id:
        return {"messages": 0, "tools": 0, "skipped": True}

    project_path = session_info.get("_project_path", "")
    created = session_info.get("created", "")
    modified = session_info.get("modified", "")
    git_branch = session_info.get("gitBranch", "")

    # Find transcript file
    full_path = session_info.get("fullPath", "")
    if full_path:
        jsonl_path = Path(full_path)
    else:
        project_dir = session_info.get("_project_dir", "")
        jsonl_path = Path(project_dir) / f"{session_id}.jsonl"

    # Insert session
    conn.execute(
        """INSERT INTO sessions (session_id, cwd, start_time, end_time, source, transcript_path)
           VALUES (?, ?, ?, ?, ?, ?)
           ON CONFLICT (session_id) DO UPDATE SET
               cwd = CASE WHEN sessions.cwd = 'unknown' AND excluded.cwd != 'unknown' THEN excluded.cwd ELSE sessions.cwd END,
               end_time = COALESCE(excluded.end_time, sessions.end_time),
               updated_at = datetime('now')""",
        (
            session_id,
            project_path or "unknown",
            created or datetime.now(timezone.utc).isoformat(),
            modified or None,
            "ingest",
            str(jsonl_path) if jsonl_path.exists() else None,
        ),
    )

    # Detect agent sessions and remap user roles accordingly
    agent_type = detect_agent_type(project_path)

    # Parse transcript
    transcript = parse_transcript(jsonl_path, agent_type=agent_type)
    messages = transcript["messages"]
    tool_usages = transcript["tool_usages"]

    msg_count = 0
    tool_count = 0

    # Build a map from message sequence_num to database message id
    msg_id_map = {}

    for msg in messages:
        cursor = conn.execute(
            """INSERT INTO messages
                   (session_id, role, content, sequence_num, timestamp,
                    model, input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT (session_id, sequence_num) DO UPDATE SET
                   role = excluded.role,
                   content = excluded.content,
                   model = COALESCE(excluded.model, messages.model),
                   input_tokens = COALESCE(excluded.input_tokens, messages.input_tokens),
                   output_tokens = COALESCE(excluded.output_tokens, messages.output_tokens)
               RETURNING id""",
            (
                session_id,
                msg["role"],
                msg["content"],
                msg["sequence_num"],
                msg["timestamp"] or None,
                msg["model"] or None,
                msg["input_tokens"],
                msg["output_tokens"],
                msg["cache_read_tokens"],
                msg["cache_creation_tokens"],
            ),
        )
        row = cursor.fetchone()
        if row:
            msg_id_map[msg["sequence_num"]] = row[0]
            msg_count += 1

    # Insert tool usages
    for i, tool in enumerate(tool_usages):
        message_id = msg_id_map.get(tool["message_index"])
        if not message_id:
            continue

        conn.execute(
            """INSERT INTO tool_usages (message_id, tool_name, tool_input, sequence_num)
               VALUES (?, ?, ?, ?)
               ON CONFLICT DO NOTHING""",
            (message_id, tool["tool_name"], tool["tool_input"], i),
        )
        tool_count += 1

    return {"messages": msg_count, "tools": tool_count, "skipped": False}


def import_stats(conn: sqlite3.Connection, claude_dir: Path):
    """Import stats-cache.json into daily_usage, model_usage, and overall_stats."""
    stats_path = claude_dir / "stats-cache.json"
    if not stats_path.exists():
        print("No stats-cache.json found, skipping stats import.", file=sys.stderr)
        return

    try:
        with open(stats_path) as f:
            data = json.load(f)
    except (json.JSONDecodeError, OSError) as e:
        print(f"Error reading stats-cache.json: {e}", file=sys.stderr)
        return

    # Daily usage from dailyModelTokens
    daily_tokens = data.get("dailyModelTokens", [])
    daily_activity = {d["date"]: d for d in data.get("dailyActivity", [])}

    for day in daily_tokens:
        date = day.get("date", "")
        tokens_by_model = day.get("tokensByModel", {})
        activity = daily_activity.get(date, {})

        for model, total_tokens in tokens_by_model.items():
            conn.execute(
                """INSERT INTO daily_usage (date, model, output_tokens, message_count, session_count, tool_call_count)
                   VALUES (?, ?, ?, ?, ?, ?)
                   ON CONFLICT (date, model) DO UPDATE SET
                       output_tokens = excluded.output_tokens,
                       message_count = excluded.message_count,
                       session_count = excluded.session_count,
                       tool_call_count = excluded.tool_call_count,
                       synced_at = datetime('now')""",
                (
                    date,
                    model,
                    total_tokens,
                    activity.get("messageCount", 0),
                    activity.get("sessionCount", 0),
                    activity.get("toolCallCount", 0),
                ),
            )

    # Model usage
    model_usage = data.get("modelUsage", {})
    for model, usage in model_usage.items():
        conn.execute(
            """INSERT INTO model_usage
                   (model, input_tokens, output_tokens, cache_read_tokens,
                    cache_creation_tokens, web_search_requests)
               VALUES (?, ?, ?, ?, ?, ?)
               ON CONFLICT (model) DO UPDATE SET
                   input_tokens = excluded.input_tokens,
                   output_tokens = excluded.output_tokens,
                   cache_read_tokens = excluded.cache_read_tokens,
                   cache_creation_tokens = excluded.cache_creation_tokens,
                   web_search_requests = excluded.web_search_requests,
                   synced_at = datetime('now')""",
            (
                model,
                usage.get("inputTokens", 0),
                usage.get("outputTokens", 0),
                usage.get("cacheReadInputTokens", 0),
                usage.get("cacheCreationInputTokens", 0),
                usage.get("webSearchRequests", 0),
            ),
        )

    # Overall stats
    hour_counts = data.get("hourCounts", {})
    # Convert dict {"0": N, "1": N, ...} to list of 24 ints
    hour_list = [hour_counts.get(str(i), 0) for i in range(24)]

    conn.execute(
        """INSERT INTO overall_stats (id, total_sessions, total_messages,
               longest_session_messages, hour_counts, stats_cache_date)
           VALUES (1, ?, ?, ?, ?, ?)
           ON CONFLICT (id) DO UPDATE SET
               total_sessions = excluded.total_sessions,
               total_messages = excluded.total_messages,
               longest_session_messages = excluded.longest_session_messages,
               hour_counts = excluded.hour_counts,
               stats_cache_date = excluded.stats_cache_date,
               synced_at = datetime('now')""",
        (
            data.get("totalSessions", 0),
            data.get("totalMessages", 0),
            data.get("longestSession", {}).get("messageCount", 0),
            json.dumps(hour_list),
            data.get("lastComputedDate", ""),
        ),
    )

    conn.commit()
    print(f"Stats imported: {len(model_usage)} models, {len(daily_tokens)} daily entries")


def main():
    parser = argparse.ArgumentParser(
        description="Import Claude Code history into dashboard-native SQLite database."
    )
    parser.add_argument(
        "--path",
        type=Path,
        default=DEFAULT_CLAUDE_DIR,
        help=f"Path to .claude directory (default: {DEFAULT_CLAUDE_DIR})",
    )
    parser.add_argument(
        "--db",
        type=Path,
        default=DEFAULT_DB_PATH,
        help=f"Path to SQLite database (default: {DEFAULT_DB_PATH})",
    )
    parser.add_argument(
        "--since",
        type=str,
        default=None,
        help="Only import sessions created within this period (e.g. 7d, 24h, 30m)",
    )
    parser.add_argument(
        "--stats-only",
        action="store_true",
        help="Only import stats-cache.json, skip session transcripts",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Show what would be imported without writing to database",
    )

    args = parser.parse_args()

    claude_dir = args.path.expanduser()
    if not claude_dir.exists():
        print(f"Error: Claude directory not found: {claude_dir}", file=sys.stderr)
        sys.exit(1)

    since = parse_since(args.since) if args.since else None

    print(f"Claude dir: {claude_dir}")
    print(f"Database:   {args.db}")
    if since:
        print(f"Since:      {since.strftime('%Y-%m-%d %H:%M')}")
    print()

    if args.dry_run:
        sessions = discover_sessions(claude_dir, since)
        if not sessions:
            print("No Claude Code sessions found.")
            print()
            print("To generate history, use Claude Code: https://claude.ai/claude-code")
            print(f"Or specify a custom path: python3 ingest.py --path /path/to/.claude/")
            return
        print(f"Would import {len(sessions)} sessions")
        for s in sessions[:10]:
            print(f"  {s['sessionId'][:8]}... {s.get('summary', s.get('firstPrompt', '')[:60])}")
        if len(sessions) > 10:
            print(f"  ... and {len(sessions) - 10} more")
        return

    conn = init_db(args.db)

    # Import stats
    import_stats(conn, claude_dir)

    if args.stats_only:
        conn.close()
        return

    # Discover and import sessions
    sessions = discover_sessions(claude_dir, since)
    if not sessions:
        print("No Claude Code sessions found.")
        print()
        print("To generate history, use Claude Code: https://claude.ai/claude-code")
        print(f"Or specify a custom path: python3 ingest.py --path /path/to/.claude/")
        conn.close()
        return
    print(f"Found {len(sessions)} sessions to import")
    print()

    total_msgs = 0
    total_tools = 0
    imported = 0
    errors = 0

    for i, session in enumerate(sessions):
        session_id = session.get("sessionId", "unknown")
        summary = session.get("summary", session.get("firstPrompt", ""))
        label = (summary or "")[:50]

        try:
            result = import_session(conn, session, claude_dir)
            if result["skipped"]:
                continue

            total_msgs += result["messages"]
            total_tools += result["tools"]
            imported += 1

            if (i + 1) % 50 == 0 or i == len(sessions) - 1:
                conn.commit()

            print(
                f"  [{i+1}/{len(sessions)}] {session_id[:8]}... "
                f"({result['messages']} msgs, {result['tools']} tools) {label}"
            )

        except Exception as e:
            errors += 1
            print(f"  [{i+1}/{len(sessions)}] {session_id[:8]}... ERROR: {e}", file=sys.stderr)

    conn.commit()
    conn.close()

    print()
    print(f"Done: {imported} sessions, {total_msgs} messages, {total_tools} tool usages")
    if errors:
        print(f"  {errors} errors encountered")


if __name__ == "__main__":
    main()
