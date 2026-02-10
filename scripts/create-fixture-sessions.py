#!/usr/bin/env python3
"""Create synthetic Claude Code session data for smoke testing ingestion.

Generates a minimal but realistic ~/.claude/ directory structure with
sessions, messages, tool usages, and stats — enough to verify ingest.py
works end-to-end.
"""

import json
import os
import sys
from datetime import datetime, timedelta

CLAUDE_DIR = os.path.expanduser("~/.claude")
PROJECT_DIR_NAME = "-Users-testuser-projects-myapp"


def create_session(session_id: str, start: datetime, messages: list[dict]) -> str:
    """Build a .jsonl transcript from a list of message specs."""
    lines = []
    cwd = "/Users/testuser/projects/myapp"

    for i, msg in enumerate(messages):
        ts = (start + timedelta(seconds=i * 10)).strftime("%Y-%m-%dT%H:%M:%S.000Z")

        if msg["role"] == "user":
            lines.append(json.dumps({
                "type": "user",
                "timestamp": ts,
                "sessionId": session_id,
                "cwd": cwd,
                "message": {
                    "role": "user",
                    "content": msg["content"],
                },
            }))
        elif msg["role"] == "assistant":
            content_blocks = [{"type": "text", "text": msg["content"]}]

            # Add tool usage if specified
            for tool in msg.get("tools", []):
                content_blocks.append({
                    "type": "tool_use",
                    "name": tool["name"],
                    "id": f"toolu_test_{i}",
                    "input": tool.get("input", {}),
                })

            lines.append(json.dumps({
                "type": "assistant",
                "timestamp": ts,
                "sessionId": session_id,
                "message": {
                    "role": "assistant",
                    "model": "claude-sonnet-4-5-20250929",
                    "content": content_blocks,
                    "usage": {
                        "input_tokens": 100,
                        "output_tokens": 50,
                        "cache_read_input_tokens": 500,
                        "cache_creation_input_tokens": 200,
                    },
                },
            }))

    return "\n".join(lines) + "\n"


def main():
    project_dir = os.path.join(CLAUDE_DIR, "projects", PROJECT_DIR_NAME)
    os.makedirs(project_dir, exist_ok=True)

    now = datetime.utcnow()

    # --- Session 1: simple Q&A ---
    s1_id = "aaaaaaaa-1111-1111-1111-aaaaaaaaaaaa"
    s1_start = now - timedelta(hours=2)
    s1_jsonl = create_session(s1_id, s1_start, [
        {"role": "user", "content": "How do I create a Python virtual environment?"},
        {"role": "assistant", "content": "You can create a virtual environment with `python3 -m venv .venv` and activate it with `source .venv/bin/activate`."},
        {"role": "user", "content": "How do I install packages?"},
        {"role": "assistant", "content": "Use `pip install package-name` or `pip install -r requirements.txt` for a requirements file."},
    ])

    # --- Session 2: coding with tool use ---
    s2_id = "bbbbbbbb-2222-2222-2222-bbbbbbbbbbbb"
    s2_start = now - timedelta(hours=1)
    s2_jsonl = create_session(s2_id, s2_start, [
        {"role": "user", "content": "Add a health check endpoint to my Flask app"},
        {
            "role": "assistant",
            "content": "I'll add a health check endpoint to your app.",
            "tools": [
                {"name": "Read", "input": {"file_path": "/app/main.py"}},
            ],
        },
        {"role": "user", "content": "Looks good, commit it"},
        {
            "role": "assistant",
            "content": "Done. I've committed the health check endpoint.",
            "tools": [
                {"name": "Bash", "input": {"command": "git add . && git commit -m 'Add health check'"}},
            ],
        },
    ])

    # Write .jsonl files
    for sid, content in [(s1_id, s1_jsonl), (s2_id, s2_jsonl)]:
        path = os.path.join(project_dir, f"{sid}.jsonl")
        with open(path, "w") as f:
            f.write(content)

    # Write sessions-index.json
    index = {
        "version": 1,
        "originalPath": "/Users/testuser/projects/myapp",
        "entries": [
            {
                "sessionId": s1_id,
                "fullPath": os.path.join(project_dir, f"{s1_id}.jsonl"),
                "fileMtime": int(s1_start.timestamp() * 1000),
                "firstPrompt": "How do I create a Python virtual environment?",
                "summary": "Python venv setup help",
                "messageCount": 4,
                "created": s1_start.strftime("%Y-%m-%dT%H:%M:%S.000Z"),
                "modified": (s1_start + timedelta(minutes=5)).strftime("%Y-%m-%dT%H:%M:%S.000Z"),
                "projectPath": "/Users/testuser/projects/myapp",
                "isSidechain": False,
            },
            {
                "sessionId": s2_id,
                "fullPath": os.path.join(project_dir, f"{s2_id}.jsonl"),
                "fileMtime": int(s2_start.timestamp() * 1000),
                "firstPrompt": "Add a health check endpoint to my Flask app",
                "summary": "Flask health check endpoint",
                "messageCount": 4,
                "created": s2_start.strftime("%Y-%m-%dT%H:%M:%S.000Z"),
                "modified": (s2_start + timedelta(minutes=5)).strftime("%Y-%m-%dT%H:%M:%S.000Z"),
                "projectPath": "/Users/testuser/projects/myapp",
                "isSidechain": False,
            },
        ],
    }
    with open(os.path.join(project_dir, "sessions-index.json"), "w") as f:
        json.dump(index, f, indent=2)

    # Write stats-cache.json
    today = now.strftime("%Y-%m-%d")
    stats = {
        "version": 2,
        "lastComputedDate": today,
        "dailyActivity": [
            {"date": today, "messageCount": 8, "sessionCount": 2, "toolCallCount": 3},
        ],
        "dailyModelTokens": [
            {"date": today, "tokensByModel": {"claude-sonnet-4-5-20250929": 800}},
        ],
        "modelUsage": {
            "claude-sonnet-4-5-20250929": {
                "inputTokens": 400,
                "outputTokens": 200,
                "cacheReadInputTokens": 2000,
                "cacheCreationInputTokens": 800,
                "webSearchRequests": 0,
            },
        },
        "totalSessions": 2,
        "totalMessages": 8,
        "longestSession": {"messageCount": 4},
        "hourCounts": {str(h): (4 if h == now.hour else 0) for h in range(24)},
    }
    with open(os.path.join(CLAUDE_DIR, "stats-cache.json"), "w") as f:
        json.dump(stats, f, indent=2)

    print(f"Created fixture data at {CLAUDE_DIR}")
    print(f"  Project: {project_dir}")
    print(f"  Sessions: {s1_id}, {s2_id}")
    print(f"  Stats: {os.path.join(CLAUDE_DIR, 'stats-cache.json')}")


if __name__ == "__main__":
    main()
