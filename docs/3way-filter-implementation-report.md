# 3-Way Filter Toggle — Implementation Blueprint

> **Status**: All changes were reverted. This document is a blueprint for re-implementation, including lessons learned from the first attempt.

## Goal

Replace the three node filters (tool uses, importance, project) — which currently each work differently — with a unified 3-way toggle: **Off / Inactive / Filtered**.

| Mode | Behavior |
|------|----------|
| **Off** | Filter disabled, all nodes visible |
| **Inactive** (Dim) | Nodes hidden from rendering, but bypass edges maintain graph connectivity |
| **Filtered** (Hide) | Nodes + all edges truly removed from the graph |

Currently:
- `hide_tool_uses: bool` — only has "Inactive" behavior (bypass edges)
- `importance_filter_enabled: bool` — only has "Filtered" behavior (nodes vanish)
- `project_filter_enabled: bool` — only has "Filtered" behavior (nodes vanish)

## Files to Modify

| File | Role |
|------|------|
| `src/graph/types.rs` | Data types — add `FilterMode` enum |
| `src/app.rs` | All UI + rendering + filter logic (~5900 lines) |
| `src/settings.rs` | Persistence + presets |

## Implementation Steps

### Step 1: `FilterMode` enum (types.rs)

Add before `ColorMode`:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum FilterMode {
    #[default]
    Off,
    Inactive,  // Nodes hidden, bypass edges maintain connectivity
    Filtered,  // Nodes + all edges truly removed
}

impl FilterMode {
    pub fn is_active(&self) -> bool { *self != FilterMode::Off }
    pub fn label(&self) -> &'static str {
        match self { Self::Off => "Off", Self::Inactive => "Dim", Self::Filtered => "Hide" }
    }
}
```

### Step 2: Replace struct fields (app.rs `DashboardApp`)

| Old Field (line) | New Field |
|------------------|-----------|
| `importance_filter_enabled: bool` (197) | `importance_filter: FilterMode` |
| `hide_tool_uses: bool` (205) | `tool_use_filter: FilterMode` |
| `tool_use_bypass_edges: Vec<GraphEdge>` (206) | `bypass_edges: Vec<GraphEdge>` |
| `project_filter_enabled: bool` (209) | `project_filter: FilterMode` |

Update the `new()` initializer (~line 423-433) accordingly.

### Step 3: Add new methods (app.rs)

**`collect_filter_sets()`** — returns `(HashSet<String>, HashSet<String>)` of (inactive_ids, filtered_ids):
- Iterates all nodes
- For each active filter, checks if node fails it, puts ID into inactive or filtered set based on FilterMode
- A node matched by the first filter gets categorized and skipped (first-match-wins)

**`compute_bypass_edges(&inactive, &filtered)`** — generalization of old `compute_tool_use_bypass_edges()`:
- Walks session chains (non-temporal, non-similarity edges)
- Bridges over `inactive_ids` nodes
- Stops chain at `filtered_ids` nodes (no bypass across truly removed nodes)
- Returns `Vec<GraphEdge>`

**`recompute_bypass_edges()`** — convenience wrapper:
```rust
fn recompute_bypass_edges(&mut self) {
    let (inactive, filtered) = self.collect_filter_sets();
    self.bypass_edges = self.compute_bypass_edges(&inactive, &filtered);
}
```

**`is_node_hidden(node_id)`** — returns true if node fails any active filter:
- Checks tool_use_filter, importance_filter, project_filter
- Does NOT check timeline or semantic filters (those are handled separately)

### Step 4: Update all filter check sites (app.rs)

These locations have 3 separate inline checks that should use `is_node_hidden()`:

| Location | Current Code | Change To |
|----------|-------------|-----------|
| `compute_physics_visible_nodes` (~1095-1133) | 3 separate `if` blocks | `is_node_hidden()` |
| `get_visible_node_ids` (~1555-1577) | 3 separate `if` blocks | `is_node_hidden()` |
| Edge rendering loop (~3781-3857) | 3 separate endpoint checks | `is_node_hidden()` on both endpoints |
| Bypass edge rendering (~3921) | `if self.hide_tool_uses` guard | `if !self.bypass_edges.is_empty()` |
| Hover detection (~3983-4035) | 3 separate checks | `is_node_hidden()` |
| Node rendering (~4072-4127) | 3 separate checks | `is_node_hidden()` |

**Bug fix**: Remove the `!edge.is_temporal && !edge.is_similarity` exemption at ~line 3851. This was allowing edges to invisible nodes to be drawn. All edges to hidden nodes should be suppressed uniformly.

### Step 5: Update UI widgets (app.rs `render_sidebar_filters`)

Replace each checkbox with a 3-button horizontal toggle:

```rust
ui.horizontal(|ui| {
    let mut mode_changed = false;
    for &mode in &[FilterMode::Off, FilterMode::Inactive, FilterMode::Filtered] {
        if ui.selectable_label(self.importance_filter == mode, mode.label()).clicked() {
            self.importance_filter = mode;
            mode_changed = true;
        }
    }
    if mode_changed {
        self.recompute_bypass_edges();
        self.semantic_filter_cache = None;
        self.mark_settings_dirty();
    }
});
```

Apply to all three: **Importance** (~3128), **Project** (~3195), **Tool Uses** (~3235).

Show sub-controls (slider, checkboxes, count) when `filter.is_active()` (same as before, just checking the enum).

### Step 6: Settings persistence (settings.rs)

**Both `Settings` and `Preset` structs** — add:
```rust
#[serde(default)]
pub importance_filter: FilterMode,
#[serde(default)]
pub tool_use_filter: FilterMode,
#[serde(default)]
pub project_filter: FilterMode,
```

Keep old `importance_filter_enabled: bool` with `#[serde(default)]` for backward compat.

**Migration in `Settings::load()`**: After deserializing, if old bool is true and new enum is Off, set to `FilterMode::Filtered`.

**Update**: `from_settings()`, `apply_to()`, `sync_settings_from_ui()`, `sync_ui_from_settings()`.

### Step 7: Histogram drill-down (app.rs)

Lines ~4779-4848 assign `project_filter_enabled = true/false`. Change to:
- `self.project_filter = FilterMode::Filtered` (when drilling into a project)
- `self.project_filter = FilterMode::Off` (when clearing)

### Step 8: `load_graph()` bypass recomputation

Call `self.recompute_bypass_edges()` **after** the `match db.fetch_graph(...)` block closes (not inside it — that causes a borrow conflict with `self.db`).

## Lessons from First Attempt

### 1. Don't use `replace_all` for bool→enum renames
`replace_all` on `self.project_filter_enabled` also replaces **assignment sites** like `= false` → `.is_active() = false`, which is invalid. Handle reads (`if self.x`) and writes (`self.x = ...`) separately.

### 2. `checkbox(&mut field)` doesn't work with enums
`ui.checkbox(&mut self.importance_filter_enabled, ...)` can't become `ui.checkbox(&mut self.importance_filter.is_active(), ...)` — you can't take `&mut` on a method return. The UI sections need full rewrites to the toggle pattern.

### 3. Borrow checker in `load_graph()`
`let Some(ref db) = self.db` holds an immutable borrow through the entire match arm. Any `&mut self` call inside that arm fails. Place `recompute_bypass_edges()` after the match block.

### 4. Both `Preset` and `Settings` have identical field patterns
Be careful with find-and-replace — both structs have the same filtering fields. Use enough surrounding context to target the right one.

## Pitfalls to Watch For

1. **`is_node_hidden` doesn't cover timeline or semantic filters** — those are checked separately in each rendering path. Don't remove those checks.

2. **Bypass edge recomputation triggers** — must call `recompute_bypass_edges()` on: mode change, threshold slider change (when Inactive), project checkbox change (when Inactive), data reload.

3. **`collect_filter_sets` uses first-match-wins** — a node matching multiple filters gets categorized by the first one. Order: tool_use → importance → project.

4. **Backward compat for settings.json** — old files have `importance_filter_enabled: true` but no `importance_filter`. Migration in `load()` is required.

5. **Histogram drill-down always uses `Filtered`** — bypass edges don't make sense for "show only this project" drilling.

6. **Temporal/similarity edge exemption removal** — old code let temporal/similarity edges through to hidden tool-use nodes. Removing this is a behavior change; test that the graph still looks right.

7. **`session_filter` and semantic filters are NOT part of this system** — they remain separate.

## Target Architecture

```
FilterMode enum (types.rs)
    Off / Inactive / Filtered

DashboardApp fields (app.rs):
    importance_filter: FilterMode
    tool_use_filter: FilterMode
    project_filter: FilterMode
    bypass_edges: Vec<GraphEdge>     // unified, computed from all Inactive-mode filters

Key methods:
    collect_filter_sets() → (HashSet<inactive>, HashSet<filtered>)
    compute_bypass_edges(inactive, filtered) → Vec<GraphEdge>
    recompute_bypass_edges()  // convenience: collect + compute + store
    is_node_hidden(node_id) → bool  // true if any active filter hides this node

Settings persistence:
    Settings + Preset both have importance_filter/tool_use_filter/project_filter: FilterMode
    Old importance_filter_enabled: bool kept for serde compat, migrated in load()

UI: 3-button horizontal toggle per filter using selectable_label
```
