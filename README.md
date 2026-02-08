# Claude Activity Dashboard (Native)

Native desktop dashboard for visualizing your Claude Code conversation history. Built with Rust + egui for smooth 60fps graph visualization with thousands of nodes.

![Dashboard Screenshot](docs/screenshot.png)

## Quickstart

```bash
git clone https://github.com/clayarnold/dashboard-native.git
cd dashboard-native
python3 ingest.py          # Import your Claude Code history
cargo run --release        # Launch the dashboard
```

That's it. No database server needed -- data is stored in a local SQLite file at `~/.config/dashboard-native/dashboard.db`, auto-created on first run.

## Requirements

- **Rust** -- Install via [rustup](https://rustup.rs)
- **Python 3.10+** -- For the API backend and data ingestion

```bash
pip install -r api/requirements.txt
```

## Features

- **Force-directed graph** -- Interactive visualization of conversations with adjustable physics
- **Timeline scrubber** -- Playback conversations over time, filter by date range
- **Session summaries** -- AI-generated summaries (optional, requires API key)
- **Importance scoring** -- Messages scored 0.0-1.0 based on decision impact
- **Semantic filters** -- Natural-language search across all messages
- **Project detection** -- Auto-groups sessions by working directory

## Data Import

The dashboard reads from Claude Code's local history at `~/.claude/`.

```bash
python3 ingest.py                          # Import all sessions
python3 ingest.py --since 7d              # Last 7 days only
python3 ingest.py --path /other/.claude/  # Custom Claude dir
python3 ingest.py --dry-run               # Preview without writing
```

Re-running is safe -- the importer is idempotent (uses `ON CONFLICT`).

## Running the Dashboard

The easiest way is the startup script, which launches both the API and the Rust app:

```bash
./start.sh
```

Or run components separately:

```bash
# Terminal 1: API server
cd api && python3 -m uvicorn main:app --host 127.0.0.1 --port 8000

# Terminal 2: Desktop app
cargo run --release
```

See `make help` for all available commands.

## AI Features (Optional)

Session summaries, importance scoring, and semantic filters use an LLM. Set one of these environment variables to enable them:

| Variable | Provider | Default Model |
|----------|----------|---------------|
| `OPENAI_API_KEY` | OpenAI | gpt-4o-mini |
| `ANTHROPIC_API_KEY` | Anthropic | claude-sonnet-4-5-20250929 |
| `GOOGLE_API_KEY` | Google Gemini | gemini-2.5-flash |

Override the model with `LLM_MODEL` or force a provider with `LLM_PROVIDER`.

If no key is set, the dashboard works normally -- AI features are simply disabled.

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `DB_PATH` | `~/.config/dashboard-native/dashboard.db` | SQLite database location |
| `LLM_PROVIDER` | auto-detected | Force: `openai`, `anthropic`, `google`, `litellm` |
| `LLM_MODEL` | provider default | Override the LLM model |
| `LITELLM_BASE_URL` | -- | LiteLLM proxy URL (legacy) |

## Architecture

```
+-----------------------------------------------------------+
|                 Rust Desktop App (egui)                    |
|  Sidebar | Force-directed graph | Timeline scrubber       |
+------------------------------+----------------------------+
                               |  HTTP (localhost:8000)
                    +----------v-----------+
                    |  Python API (FastAPI) |
                    +----------+-----------+
                               |
                    +----------v-----------+
                    |       SQLite DB       |
                    |  ~/.config/dashboard- |
                    |  native/dashboard.db  |
                    +-----------------------+
```

**Data flow:**
1. `ingest.py` reads `~/.claude/` history into SQLite
2. Python API serves graph/session/metric queries over HTTP
3. Rust app fetches data, renders interactive visualization

## File Structure

```
dashboard-native/
  Cargo.toml               Rust dependencies
  Makefile                  Build/run commands
  ingest.py                 Import Claude Code history
  schema.sqlite.sql         Database schema
  start.sh                  One-command startup
  src/
    main.rs                 Entry point
    app.rs                  UI rendering (~700 lines)
    db.rs                   Direct SQLite access
    settings.rs             User preferences
    theme.rs                Visual styling
    graph/
      types.rs              Data structures, TimelineState
      layout.rs             Force-directed physics
    api/
      client.rs             HTTP client for Python API
  api/
    main.py                 FastAPI endpoints
    requirements.txt        Python dependencies
    project_detection.py    Auto-detect projects from paths
    db/
      queries.py            SQL queries (SQLite)
      llm.py                Multi-provider LLM client
      summarizer.py         AI session summaries
      importance/           Importance scoring pipeline
      semantic_filters.py   Natural-language filters
  .github/workflows/
    ci.yml                  PR checks (cargo check/clippy/test)
    release.yml             Cross-platform release builds
```

## Development

```bash
# Debug build (faster compilation)
cargo build

# Release build (optimized, ~10x faster)
cargo build --release

# Run tests
cargo test
```

## Performance

| Metric | Value |
|--------|-------|
| FPS (500 nodes) | ~60 |
| FPS (1000 nodes) | ~55 |
| Startup time | ~0.5s |
| Memory | ~50MB |

## License

MIT
