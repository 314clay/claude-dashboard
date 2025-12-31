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
cd api && /Users/clayarnold/w/connect/venv/bin/uvicorn main:app --port 8000 &
source ~/.cargo/env && cargo run --release
```

## Common Tasks

### Add a new sidebar control

Edit `src/app.rs`, find `fn render_sidebar`, add UI element:
```rust
ui.checkbox(&mut self.some_flag, "Label");
// or
ui.add(egui::Slider::new(&mut self.value, 0.0..=100.0).text("Label"));
```

### Add a new node type

1. Edit `src/graph/types.rs`, add variant to `Role` enum
2. Add color in `Role::color()`
3. Add label in `Role::label()`
4. Update legend in `render_sidebar`

### Add a new API endpoint

1. Edit `api/main.py`, add new route
2. Edit `src/api/client.rs`, add fetch method
3. Call from `src/app.rs`

### Filter nodes/edges by some criteria

In `render_graph`, add condition before drawing:
```rust
for node in &self.graph.data.nodes {
    if some_condition {
        continue;  // Skip this node
    }
    // ... draw node
}
```

## Rust Patterns Used

### Borrow Checker Workaround
When you need to read from `self.graph` and also call `self.graph.update_something()`:
```rust
// Cache values BEFORE any closures
let value = self.graph.some_field;
let other = self.graph.timeline.position;

ui.horizontal(|ui| {
    // Now safe to use cached values and mutate self
    if ui.button("Click").clicked() {
        self.graph.update_something();
    }
});
```

### Custom Painter Drawing
```rust
let (response, painter) = ui.allocate_painter(size, egui::Sense::click_and_drag());
let rect = response.rect;

// Draw shapes
painter.rect_filled(rect, corner_radius, color);
painter.circle_filled(center, radius, color);
painter.line_segment([p1, p2], Stroke::new(width, color));
```

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

## Next Features to Implement

### Detail Panel (US-6.4)
- Add `detail_panel_open: bool` and `detail_node_id: Option<String>` to app state
- On double-click, set these
- Add `egui::SidePanel::right("detail")` in update()
- Show: role, full content, session info, connections

### Obsidian Nodes (US-6.1)
- API already has `get_obsidian_notes_with_links()` in queries.py
- Add endpoint to api/main.py
- Fetch on load, merge into graph data
- Draw as diamond shape instead of circle

### Keyboard Shortcuts
```rust
// In update():
if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
    self.graph.timeline.playing = !self.graph.timeline.playing;
}
```

## Performance Notes

- egui repaints only when `ctx.request_repaint()` is called
- We call it when physics is running or timeline is playing
- FPS counter in sidebar shows actual frame rate
- Release build (`--release`) is ~10x faster than debug

## Database

PostgreSQL on port 5433, schema `claude_sessions`:
- `sessions`: session_id, cwd, start_time, end_time
- `messages`: id, session_id, role, content, timestamp, sequence_num
- `tool_usages`: tool_name, tool_input, timestamp, message_id
- `session_summaries`: summary, topics, detected_project
- `session_embeddings`: embedding (pgvector)
