# Dashboard Native - Development Notes

## Project Context

This is a native Rust rewrite of the Claude Activity Dashboard. The original Streamlit + vis.js version struggled with 500+ nodes. This version uses egui for native performance.

**Decision**: We chose Rust + egui over Tauri + Cosmograph because:
1. True native performance (no WebGL issues documented with Tauri)
2. Single language for the whole app
3. Smallest memory footprint (~50MB vs 200MB+ for Electron)

**Tradeoff**: Python API for data layer (reuses existing queries) rather than rewriting in Rust with sqlx.

## Key Files

| File | Purpose | Lines |
|------|---------|-------|
| `src/app.rs` | Main UI, all rendering, timeline | ~700 |
| `src/graph/types.rs` | Data structures, TimelineState | ~400 |
| `src/graph/layout.rs` | Force-directed physics | ~100 |
| `src/api/client.rs` | HTTP client to Python API | ~70 |
| `api/main.py` | FastAPI wrapper for queries | ~90 |

## Running the App

```bash
# Full stack (recommended)
./start.sh

# Or manually:
cd api && python3 -m uvicorn main:app --port 8000 &
source ~/.cargo/env && cargo run --release
```

## Developer Guide

See `docs/DEV_GUIDE.md` for common task recipes, Rust patterns, importance scoring details, and planned features.

## Timeline Implementation

The timeline works by:
1. On data load, `build_timeline()` sorts nodes by timestamp
2. `TimelineState` tracks `position` (0.0-1.0) and `start_position`
3. `update_visible_nodes()` populates `visible_nodes: HashSet<String>`
4. `render_graph` checks `is_node_visible()` / `is_edge_visible()` before drawing
5. Playback advances `position` each frame based on `speed`

## Known Issues / Tech Debt

1. **Timestamps**: Manual ISO8601 parsing in `GraphNode::timestamp_secs()`. Could use `chrono` crate but wanted to avoid dependencies.
2. **Unused fields**: Several struct fields have `#[warn(dead_code)]` warnings. These are for future features (detail panel, similarity edges).
3. **Physics on hidden nodes**: Force simulation still runs on all nodes even when timeline hides some. Could optimize by only simulating visible nodes.

## Performance Notes

- egui repaints only when `ctx.request_repaint()` is called
- We call it when physics is running or timeline is playing
- FPS counter in sidebar shows actual frame rate
- Release build (`--release`) is ~10x faster than debug

## Database

SQLite at `~/.config/dashboard-native/dashboard.db`:
- `sessions`: session_id, cwd, start_time, end_time
- `messages`: id, session_id, role, content, timestamp, sequence_num, importance_score
- `tool_usages`: tool_name, tool_input, timestamp, message_id
- `session_summaries`: summary, topics, detected_project
- `session_contexts`: session_id, summary, completed_work, topics (for importance scoring)
- `daily_usage`: date, model, token counts
- `model_usage`: per-model cumulative stats
- `overall_stats`: aggregate counts
- `semantic_filters` / `semantic_filter_results`: natural-language filters
