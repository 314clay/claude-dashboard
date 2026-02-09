# Dashboard Native - Developer Guide

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

## Importance Scoring

Scores messages 0.0-1.0 based on **decisions made**, not actions taken.

### Scoring Criteria

| Score | Category | Examples |
|-------|----------|----------|
| 0.8-1.0 | Major Decisions | Architecture choices, technology picks, "let's use X approach" |
| 0.6-0.8 | Minor Decisions | Implementation choices, API design, tradeoff resolutions |
| 0.5-0.7 | Task Definitions | Defining what to build, scoping work, requirements |
| 0.3-0.5 | Context | Background info, explanations, clarifications |
| 0.1-0.3 | Execution | Running commands, building, testing - NO decision made |
| 0.0-0.2 | Filler | "thanks", "got it", "ok", acknowledgments |

**Key insight**: Code/commands are LOW importance unless they represent a DECISION. "Run the build" = 0.2. "Let's add caching with Redis" = 0.8.

### Architecture

Two-phase approach in `dashboard/components/importance/`:

1. **Context Creation** (`context.py`): Generate session summary via LLM for scoring context
2. **Scoring** (`scorer.py`): Score messages in batches using the context

Orchestrated by `backfill.py` with parallel support via ThreadPoolExecutor.

### API Endpoints

```
GET  /importance/stats                    # Coverage statistics
POST /importance/backfill?parallel=20     # Batch scoring (parallel workers)
POST /importance/session/{id}             # Score specific session
```

### Backfill Parameters

| Param | Default | Description |
|-------|---------|-------------|
| max_sessions | 50 | Sessions to process per call |
| staleness_days | 1.0 | Days inactive before scoring |
| batch_size | 100 | Messages per LLM call |
| parallel | 1 | Concurrent workers (max ~20 for Gemini) |
| since_days | None | Only recent sessions |

### Common Issues

1. **LLM JSON parsing errors**: Reduce `batch_size` (25 is safe, 100+ can truncate)
2. **Context creation failures**: Some sessions have unusual content; skip them
3. **Token limits**: Large batches hit output token limits, causing truncated JSON

### Database Columns

Messages table additions:
- `importance_score`: float 0.0-1.0
- `importance_reason`: brief explanation (max 255 chars)
- `importance_scored_at`: timestamp
