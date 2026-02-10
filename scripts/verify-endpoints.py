#!/usr/bin/env python3
"""Verify all API endpoints respond correctly. Used by the Docker smoke test."""

import json
import subprocess
import sys
import time
import os
import signal

API_URL = "http://127.0.0.1:8000"


def fetch(path: str) -> dict:
    """Fetch a JSON endpoint via curl."""
    result = subprocess.run(
        ["curl", "-sf", f"{API_URL}{path}"],
        capture_output=True, text=True
    )
    if result.returncode != 0:
        print(f"FAIL: {path} — curl returned {result.returncode}")
        sys.exit(1)
    return json.loads(result.stdout)


def main():
    # Start the API server
    print("Starting API server...")
    api = subprocess.Popen(
        [sys.executable, "-m", "uvicorn", "main:app", "--host", "127.0.0.1", "--port", "8000"],
        cwd=os.path.join(os.path.dirname(__file__), "..", "api"),
    )

    # Wait for API to be ready
    print("Waiting for API...", end="", flush=True)
    for _ in range(30):
        try:
            result = subprocess.run(
                ["curl", "-sf", f"{API_URL}/health"],
                capture_output=True, timeout=2,
            )
            if result.returncode == 0:
                print(" ready")
                break
        except subprocess.TimeoutExpired:
            pass
        print(".", end="", flush=True)
        time.sleep(0.5)
    else:
        print(" FAILED — API did not start")
        api.kill()
        sys.exit(1)

    failures = 0

    # Health check
    print("── Health check ──")
    data = fetch("/health")
    if data == {"status": "ok"}:
        print("PASS: /health")
    else:
        print(f"FAIL: /health — got {data}")
        failures += 1

    # Graph endpoint — with ingested data we expect nodes
    print("── Graph endpoint ──")
    data = fetch("/graph?hours=24")
    if "nodes" not in data:
        print("FAIL: /graph — missing 'nodes' key")
        failures += 1
    elif len(data["nodes"]) == 0:
        print("FAIL: /graph — 0 nodes (expected ingested session data)")
        failures += 1
    else:
        print(f"PASS: /graph ({len(data['nodes'])} nodes)")

    # Sessions endpoint — expect 2 fixture sessions
    print("── Sessions endpoint ──")
    data = fetch("/sessions?hours=24")
    if "sessions" not in data:
        print("FAIL: /sessions — missing 'sessions' key")
        failures += 1
    elif len(data["sessions"]) < 2:
        print(f"FAIL: /sessions — got {len(data['sessions'])} sessions, expected >= 2")
        failures += 1
    else:
        print(f"PASS: /sessions ({len(data['sessions'])} sessions)")

    # Semantic filters endpoint
    print("── Semantic filters endpoint ──")
    data = fetch("/semantic-filters")
    if "filters" in data:
        print("PASS: /semantic-filters")
    else:
        print(f"FAIL: /semantic-filters — missing 'filters' key")
        failures += 1

    # Binary exists
    print("── Binary check ──")
    binary = "/app/target/release/dashboard-native"
    if os.path.isfile(binary) and os.access(binary, os.X_OK):
        print("PASS: release binary built")
    else:
        print(f"FAIL: binary not found at {binary}")
        failures += 1

    # Cleanup
    api.terminate()
    try:
        api.wait(timeout=5)
    except subprocess.TimeoutExpired:
        api.kill()

    print()
    if failures:
        print(f"FAILED: {failures} check(s) did not pass")
        sys.exit(1)
    else:
        print("========================================")
        print("  ALL SMOKE TESTS PASSED")
        print("========================================")


if __name__ == "__main__":
    main()
