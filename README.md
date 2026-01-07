# Claude Activity Dashboard (Native)

High-performance native desktop app for visualizing Claude Code sessions. Built with Rust + egui for smooth 60fps graph visualization with thousands of nodes.

## Features

- **Force-directed graph** - Interactive visualization of conversations with adjustable physics
- **Timeline scrubber** - Playback conversations over time, filter by date range
- **Session summaries** - AI-generated summaries using Gemini (via LiteLLM proxy)
- **Importance scoring** - Messages scored 0-1 based on decision impact

## Quick Start

### Prerequisites

- **Rust** - Install via [rustup](https://rustup.rs)
- **Python 3.10+** - For the API backend
- **PostgreSQL** - With the `claude_sessions` schema

### Setup

1. **Clone and navigate to the dashboard-native directory**

2. **Set up Python environment**
   ```bash
   cd api
   python3 -m venv ../venv
   source ../venv/bin/activate
   pip install -r requirements.txt
   ```

3. **Configure database** (copy and edit)
   ```bash
   cp api/.env.example api/.env
   # Edit api/.env with your database credentials
   ```

4. **Start the app**
   ```bash
   ./start.sh
   ```

The script will:
- Start the Python API on port 8000
- Build the Rust app (first run takes ~1 min)
- Launch the dashboard

## Configuration

Copy `api/.env.example` to `api/.env` and configure:

```bash
# Database
DB_HOST=localhost
DB_PORT=5433
DB_USER=postgres
DB_NAME=connectingservices

# LLM for AI summaries (optional)
LITELLM_BASE_URL=http://localhost:4001
LITELLM_API_KEY=sk-your-key
```

## Database Schema

The dashboard expects a PostgreSQL database with the `claude_sessions` schema:

```sql
-- Core tables
claude_sessions.sessions (session_id, cwd, start_time, end_time)
claude_sessions.messages (id, session_id, role, content, timestamp, importance_score)
claude_sessions.tool_usages (id, message_id, tool_name, tool_input, timestamp)
claude_sessions.session_summaries (session_id, summary, topics, detected_project)
```

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
├─────────────────────────────────────────────────────────┤
│                    reqwest HTTP                         │
└────────────────────────┬────────────────────────────────┘
                         │
           ┌─────────────▼────────────┐
           │   Python API (FastAPI)   │
           │   localhost:8000         │
           └─────────────┬────────────┘
                         │
           ┌─────────────▼────────────┐
           │      PostgreSQL          │
           └──────────────────────────┘
```

## Development

### Run components separately

```bash
# API only (with hot reload)
cd api
source ../venv/bin/activate
uvicorn main:app --reload --port 8000

# Rust app only (API must be running)
cargo run --release
```

### Build

```bash
# Debug build (faster compilation)
cargo build

# Release build (optimized)
cargo build --release
```

### API Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /health` | Health check |
| `GET /graph?hours=24` | Nodes and edges for graph |
| `GET /sessions?hours=24&limit=50` | Session list |
| `GET /metrics?hours=24` | Overview counts |
| `GET /session/{id}/messages` | Messages for a session |
| `GET /session/{id}/summary` | AI-generated summary |
| `GET /tools?hours=24` | Tool usage stats |
| `GET /projects` | Detected projects |
| `POST /importance/backfill` | Score message importance |

## File Structure

```
dashboard-native/
├── Cargo.toml              # Rust dependencies
├── start.sh                # One-command startup
├── README.md               # This file
│
├── src/
│   ├── main.rs             # Entry point
│   ├── app.rs              # UI rendering
│   ├── api/client.rs       # HTTP client
│   └── graph/
│       ├── types.rs        # Data structures
│       └── layout.rs       # Force-directed physics
│
└── api/
    ├── main.py             # FastAPI endpoints
    ├── requirements.txt    # Python dependencies
    ├── .env.example        # Configuration template
    ├── project_detection.py
    └── db/                 # Database module
        ├── queries.py      # SQL queries
        ├── summarizer.py   # AI summaries
        └── importance/     # Importance scoring
```

## Performance

| Metric | Target | Actual |
|--------|--------|--------|
| FPS (500 nodes) | ≥55 | ~60 |
| FPS (1000 nodes) | ≥45 | ~55 |
| Startup time | <1s | ~0.5s |
| Memory | <100MB | ~50MB |

## License

MIT
