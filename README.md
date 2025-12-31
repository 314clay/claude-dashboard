# Claude Activity Dashboard (Native)

High-performance native desktop app for visualizing Claude Code sessions. Built with Rust + egui, replacing the laggy Streamlit + vis.js implementation.

## Quick Start

```bash
./start.sh
```

This starts the Python API backend and launches the Rust app.

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                 Rust App (egui + eframe)                │
├─────────────┬───────────────────────────┬───────────────┤
│   Sidebar   │       Graph Canvas        │               │
│  - Filters  │  - Force-directed layout  │               │
│  - Stats    │  - Pan/zoom/hover         │               │
│  - Controls │  - Node selection         │               │
├─────────────┴───────────────────────────┴───────────────┤
│                   Timeline Scrubber                     │
│  - Playback controls (▶ ⏸ ⏮ ⏭)                        │
│  - Speed selector (0.5x - 8x)                           │
│  - Draggable time range handles                         │
│  - Node timestamp notches                               │
├─────────────────────────────────────────────────────────┤
│                    reqwest HTTP                         │
└────────────────────────┬────────────────────────────────┘
                         │
           ┌─────────────▼────────────┐
           │   Python API (FastAPI)   │
           │   localhost:8000         │
           │                          │
           │  Wraps existing queries  │
           │  from dashboard/         │
           └─────────────┬────────────┘
                         │
           ┌─────────────▼────────────┐
           │      PostgreSQL          │
           │      localhost:5433      │
           └──────────────────────────┘
```

## Current Features

### Graph Visualization
- [x] Force-directed layout with adjustable physics
- [x] Node colors by role (white=user, orange=Claude)
- [x] Session-colored edges
- [x] Pan (drag) and zoom (scroll wheel)
- [x] Hover tooltips with message preview
- [x] Click to select nodes
- [x] Arrow indicators on edges

### Timeline Scrubber
- [x] Bottom panel with time range display
- [x] Playback controls (play/pause/reset/skip)
- [x] Speed selector (0.5x, 1x, 2x, 4x, 8x)
- [x] Draggable scrubber with notches at node timestamps
- [x] Start/end handles for time window
- [x] Nodes filter based on time window

### Sidebar Controls
- [x] Time range selector (1h, 6h, 24h, 3d, 1w)
- [x] Node size slider
- [x] Show/hide arrows toggle
- [x] Physics enable/disable
- [x] Timeline enable/disable
- [x] Physics parameters (repulsion, attraction, centering)
- [x] Statistics display (nodes, edges, FPS)
- [x] Legend

## Development

### Prerequisites

- **Rust**: Install via `rustup` (https://rustup.rs)
- **Python**: 3.10+ with venv at `/Users/clayarnold/w/connect/venv`
- **PostgreSQL**: Running on port 5433 (via Docker)

### Build

```bash
# Debug build (faster compilation)
source ~/.cargo/env
cargo build

# Release build (optimized, what start.sh uses)
cargo build --release
```

### Run Components Separately

```bash
# API only
cd api
/Users/clayarnold/w/connect/venv/bin/uvicorn main:app --reload --port 8000

# App only (API must be running)
source ~/.cargo/env
cargo run --release
```

### API Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /health` | Health check |
| `GET /graph?hours=24` | Nodes and edges for graph |
| `GET /sessions?hours=24&limit=50` | Session list |
| `GET /metrics?hours=24` | Overview counts |
| `GET /session/{id}/messages` | Messages for a session |
| `GET /tools?hours=24` | Tool usage stats |

## File Structure

```
dashboard-native/
├── Cargo.toml              # Rust dependencies
├── start.sh                # One-command startup
├── README.md               # This file
├── CLAUDE.md               # Dev notes for Claude Code
│
├── src/
│   ├── main.rs             # Entry point, window setup
│   ├── app.rs              # UI state, rendering, timeline
│   │
│   ├── api/
│   │   ├── mod.rs          # Module exports
│   │   └── client.rs       # HTTP client for Python API
│   │
│   └── graph/
│       ├── mod.rs          # Module exports
│       ├── types.rs        # GraphNode, GraphEdge, GraphState, TimelineState
│       └── layout.rs       # Force-directed physics simulation
│
└── api/
    ├── main.py             # FastAPI app wrapping dashboard queries
    └── requirements.txt    # Python deps (fastapi, uvicorn)
```

## Performance

| Metric | Target | Actual |
|--------|--------|--------|
| FPS (500 nodes) | ≥55 | ~60 |
| FPS (1000 nodes) | ≥45 | TBD |
| Startup time | <1s | ~0.5s |
| Memory | <100MB | ~50MB |

## Roadmap

### Phase 1: Graph Prototype ✅
- [x] Basic graph rendering
- [x] Force-directed layout
- [x] Pan/zoom/hover/click
- [x] Timeline scrubber with playback

### Phase 2: Graph Polish
- [ ] Detail panel (slide-in from right on double-click)
- [ ] Obsidian node type (purple diamond)
- [ ] Topic node type (green hexagon)
- [ ] Semantic similarity edges (cyan, dashed)
- [ ] Session filter dropdown
- [ ] Keyboard shortcuts (spacebar=play, arrows=step)

### Phase 3: Additional Pages
- [ ] Overview dashboard (metrics, charts)
- [ ] Session explorer (list + detail view)
- [ ] Tools analysis (usage breakdown)
- [ ] Work patterns (heatmap, badges)

## Spec Document

Full feature spec with 31 user stories:
`/Users/clayarnold/w/connect/docs/dashboard-feature-spec.md`

## Related

- **Original Streamlit dashboard**: `/Users/clayarnold/w/connect/dashboard/`
- **Database schema**: `claude_sessions` in PostgreSQL
- **Docker config**: `/Users/clayarnold/w/connect/database/docker/docker-compose.yml`
