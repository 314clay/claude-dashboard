//! SQLite database access using sqlx.
//!
//! Self-contained local database. Creates the DB file and schema on first run.

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{FromRow, SqlitePool};
use std::sync::Arc;
use tokio::runtime::Runtime;

use crate::graph::types::{GraphData, GraphEdge, GraphNode, Role, SessionSummaryData};

/// Embedded schema — run on every connect (all statements are IF NOT EXISTS).
const SCHEMA_SQL: &str = include_str!("../schema.sqlite.sql");

/// Resolve the database file path (env override or default config dir).
/// Uses ~/.config/dashboard-native/ to match the Python ingest script.
fn db_path() -> String {
    std::env::var("DB_PATH").unwrap_or_else(|_| {
        let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        let config_dir = home.join(".config").join("dashboard-native");
        std::fs::create_dir_all(&config_dir).ok();
        config_dir.join("dashboard.db").to_string_lossy().to_string()
    })
}

/// Row returned from the graph query
#[derive(Debug, FromRow)]
struct MessageRow {
    id: i32,
    session_id: String,
    role: String,
    content: Option<String>,
    timestamp: Option<String>,
    sequence_num: i32,
    importance_score: Option<f64>,
    importance_reason: Option<String>,
    token_count: Option<i32>,
    input_tokens: Option<i32>,
    cache_read_tokens: Option<i32>,
    cache_creation_tokens: Option<i32>,
    cwd: Option<String>,
}

/// Row returned from importance stats query
#[derive(Debug, FromRow)]
struct ImportanceStatsRow {
    total_messages: Option<i64>,
    scored_messages: Option<i64>,
    unscored_messages: Option<i64>,
    sessions_with_unscored: Option<i64>,
}

/// Row returned from session summary query
#[derive(Debug, FromRow)]
struct SummaryRow {
    summary: Option<String>,
    user_requests: Option<String>,
    completed_work: Option<String>,
    topics: Option<String>,
    detected_project: Option<String>,
    generated_at: Option<String>,
}

/// Row returned from semantic filter matches query
#[derive(Debug, FromRow)]
struct FilterMatchRow {
    message_id: i64,
    filter_id: i32,
}

/// Importance scoring statistics
#[derive(Debug, Clone)]
pub struct ImportanceStats {
    pub total_messages: i64,
    pub scored_messages: i64,
    pub unscored_messages: i64,
    pub sessions_with_unscored: i64,
}

/// Database client with connection pool
pub struct DbClient {
    pool: SqlitePool,
    runtime: Arc<Runtime>,
}

impl DbClient {
    /// Create a new database client
    pub fn new() -> Result<Self, String> {
        let runtime = Runtime::new().map_err(|e| format!("Failed to create runtime: {}", e))?;

        let path = db_path();
        let url = format!("sqlite://{}?mode=rwc", path);

        let pool = runtime.block_on(async {
            let pool = SqlitePoolOptions::new()
                .max_connections(5)
                .connect(&url)
                .await
                .map_err(|e| format!("Failed to connect to database: {}", e))?;

            // Enable foreign keys and WAL mode
            sqlx::query("PRAGMA journal_mode = WAL")
                .execute(&pool)
                .await
                .ok();
            sqlx::query("PRAGMA foreign_keys = ON")
                .execute(&pool)
                .await
                .ok();

            // Run schema (all CREATE IF NOT EXISTS — safe to repeat)
            // Split on semicolons and execute each statement individually
            // because sqlx doesn't support multiple statements in one query for SQLite.
            for statement in SCHEMA_SQL.split(';') {
                // Strip SQL comment lines before checking if the segment is empty,
                // because comments and CREATE TABLE share the same ;-delimited block.
                let sql: String = statement
                    .lines()
                    .filter(|line| !line.trim().starts_with("--"))
                    .collect::<Vec<_>>()
                    .join("\n");
                let sql = sql.trim();
                if sql.is_empty() || sql.starts_with("PRAGMA") {
                    continue;
                }
                sqlx::query(sql)
                    .execute(&pool)
                    .await
                    .map_err(|e| format!("Schema init failed on: {}... — {}", &sql[..sql.len().min(60)], e))?;
            }

            Ok::<SqlitePool, String>(pool)
        })?;

        tracing::info!("Connected to SQLite at {}", path);

        Ok(Self {
            pool,
            runtime: Arc::new(runtime),
        })
    }

    /// Check if database is healthy
    pub fn health(&self) -> Result<bool, String> {
        self.runtime.block_on(async {
            sqlx::query("SELECT 1")
                .fetch_one(&self.pool)
                .await
                .map(|_| true)
                .map_err(|e| format!("Health check failed: {}", e))
        })
    }

    /// Fetch graph data (nodes and edges)
    pub fn fetch_graph(&self, hours: f32, session_id: Option<&str>) -> Result<GraphData, String> {
        self.runtime.block_on(async {
            let rows: Vec<MessageRow> = if let Some(sid) = session_id {
                sqlx::query_as(
                    r#"
                    SELECT
                        m.id,
                        m.session_id,
                        m.role,
                        m.content,
                        m.timestamp,
                        m.sequence_num,
                        m.importance_score,
                        m.importance_reason,
                        m.token_count,
                        m.input_tokens,
                        m.cache_read_tokens,
                        m.cache_creation_tokens,
                        s.cwd
                    FROM messages m
                    JOIN sessions s ON m.session_id = s.session_id
                    WHERE m.session_id = ?1
                    ORDER BY m.session_id, m.sequence_num
                    "#,
                )
                .bind(sid)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| format!("Query failed: {}", e))?
            } else {
                sqlx::query_as(
                    r#"
                    SELECT
                        m.id,
                        m.session_id,
                        m.role,
                        m.content,
                        m.timestamp,
                        m.sequence_num,
                        m.importance_score,
                        m.importance_reason,
                        m.token_count,
                        m.input_tokens,
                        m.cache_read_tokens,
                        m.cache_creation_tokens,
                        s.cwd
                    FROM messages m
                    JOIN sessions s ON m.session_id = s.session_id
                    WHERE m.timestamp >= datetime('now', '-' || CAST(?1 AS INTEGER) || ' hours')
                    ORDER BY m.session_id, m.sequence_num
                    "#,
                )
                .bind(hours as f64)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| format!("Query failed: {}", e))?
            };

            // Convert rows to nodes and edges
            let mut nodes = Vec::new();
            let mut edges = Vec::new();
            let mut prev_msg: std::collections::HashMap<String, String> = std::collections::HashMap::new();

            for row in rows {
                let msg_id = row.id.to_string();
                let session_id = row.session_id.clone();
                let content = row.content.unwrap_or_default();

                let role = match row.role.as_str() {
                    "user" => Role::User,
                    "assistant" => Role::Assistant,
                    _ => Role::User,
                };

                let content_preview = if content.chars().count() > 100 {
                    format!("{}...", content.chars().take(100).collect::<String>())
                } else {
                    content.clone()
                };

                let cwd = row.cwd.unwrap_or_default();
                let project = if let Some(home) = dirs::home_dir() {
                    let home_str = format!("{}/", home.display());
                    cwd.replace(&home_str, "~/")
                } else {
                    cwd.clone()
                };

                let ts = row.timestamp;

                nodes.push(GraphNode {
                    id: msg_id.clone(),
                    role,
                    content_preview,
                    full_content: Some(content),
                    session_id: session_id.clone(),
                    session_short: session_id[..8.min(session_id.len())].to_string(),
                    project,
                    timestamp: ts.clone(),
                    importance_score: row.importance_score.map(|v| v as f32),
                    importance_reason: row.importance_reason,
                    output_tokens: row.token_count,
                    input_tokens: row.input_tokens,
                    cache_read_tokens: row.cache_read_tokens,
                    cache_creation_tokens: row.cache_creation_tokens,
                    semantic_filter_matches: Vec::new(), // Populated below
                    has_tool_usage: false, // Populated below
                });

                // Create edge from previous message in same session
                if let Some(prev_id) = prev_msg.get(&session_id) {
                    edges.push(GraphEdge {
                        source: prev_id.clone(),
                        target: msg_id.clone(),
                        session_id: session_id.clone(),
                        timestamp: ts.clone(),
                        is_obsidian: false,
                        is_topic: false,
                        is_similarity: false,
                        is_temporal: false,
                        similarity: None,
                    });
                }

                prev_msg.insert(session_id, msg_id);
            }

            // Fetch semantic filter matches for all message IDs
            if !nodes.is_empty() {
                let message_ids: Vec<i32> = nodes.iter()
                    .filter_map(|n| n.id.parse::<i32>().ok())
                    .collect();

                if !message_ids.is_empty() {
                    // SQLite doesn't support ANY($1) with arrays.
                    // Build a dynamic IN clause with positional params.
                    let placeholders: Vec<String> = (1..=message_ids.len())
                        .map(|i| format!("?{}", i))
                        .collect();
                    let in_clause = placeholders.join(", ");
                    let sql = format!(
                        r#"
                        SELECT
                            r.message_id,
                            r.filter_id
                        FROM semantic_filter_results r
                        JOIN semantic_filters f ON r.filter_id = f.id
                        WHERE r.message_id IN ({})
                          AND r.matches = 1
                          AND f.is_active = 1
                        "#,
                        in_clause
                    );

                    let mut query = sqlx::query_as::<_, FilterMatchRow>(&sql);
                    for id in &message_ids {
                        query = query.bind(id);
                    }

                    let filter_matches: Vec<FilterMatchRow> = query
                        .fetch_all(&self.pool)
                        .await
                        .unwrap_or_default();

                    // Build message_id -> filter_ids mapping
                    let mut matches_map: std::collections::HashMap<i32, Vec<i32>> = std::collections::HashMap::new();
                    for row in filter_matches {
                        if let Ok(msg_id) = i32::try_from(row.message_id) {
                            matches_map.entry(msg_id).or_default().push(row.filter_id);
                        }
                    }

                    // Update nodes with their filter matches
                    for node in &mut nodes {
                        if let Ok(msg_id) = node.id.parse::<i32>() {
                            if let Some(filter_ids) = matches_map.get(&msg_id) {
                                node.semantic_filter_matches = filter_ids.clone();
                            }
                        }
                    }
                }
            }

            // Identify messages with tool usages
            if !nodes.is_empty() {
                let message_ids: Vec<i32> = nodes.iter()
                    .filter_map(|n| n.id.parse::<i32>().ok())
                    .collect();

                if !message_ids.is_empty() {
                    let placeholders: Vec<String> = (1..=message_ids.len())
                        .map(|i| format!("?{}", i))
                        .collect();
                    let in_clause = placeholders.join(", ");
                    let sql = format!(
                        "SELECT DISTINCT message_id FROM tool_usages WHERE message_id IN ({})",
                        in_clause
                    );

                    let mut query = sqlx::query_scalar::<_, i32>(&sql);
                    for id in &message_ids {
                        query = query.bind(id);
                    }

                    let tool_msg_ids: std::collections::HashSet<i32> = query
                        .fetch_all(&self.pool)
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .collect();

                    for node in &mut nodes {
                        if let Ok(msg_id) = node.id.parse::<i32>() {
                            if tool_msg_ids.contains(&msg_id) {
                                node.has_tool_usage = true;
                            }
                        }
                    }
                }
            }

            Ok(GraphData { nodes, edges, beads: Vec::new(), mail: Vec::new() })
        })
    }

    /// Fetch session summary from database (no generation)
    pub fn fetch_session_summary(&self, session_id: &str) -> Result<SessionSummaryData, String> {
        self.runtime.block_on(async {
            let row: Option<SummaryRow> = sqlx::query_as(
                r#"
                SELECT
                    summary,
                    user_requests,
                    completed_work,
                    topics,
                    detected_project,
                    generated_at
                FROM session_summaries
                WHERE session_id = ?1
                "#,
            )
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| format!("Query failed: {}", e))?;

            match row {
                Some(r) => {
                    let topics: Vec<String> = r.topics
                        .and_then(|v| serde_json::from_str(&v).ok())
                        .unwrap_or_default();

                    Ok(SessionSummaryData {
                        exists: true,
                        generated: false,
                        summary: r.summary,
                        user_requests: r.user_requests,
                        completed_work: r.completed_work,
                        topics: Some(topics),
                        detected_project: r.detected_project,
                        generated_at: r.generated_at,
                        error: None,
                    })
                }
                None => Ok(SessionSummaryData {
                    exists: false,
                    generated: false,
                    summary: None,
                    user_requests: None,
                    completed_work: None,
                    topics: None,
                    detected_project: None,
                    generated_at: None,
                    error: None,
                }),
            }
        })
    }

    /// Fetch importance scoring statistics
    pub fn fetch_importance_stats(&self) -> Result<ImportanceStats, String> {
        self.runtime.block_on(async {
            let row: ImportanceStatsRow = sqlx::query_as(
                r#"
                SELECT
                    COUNT(*) as total_messages,
                    SUM(CASE WHEN importance_score IS NOT NULL THEN 1 ELSE 0 END) as scored_messages,
                    SUM(CASE WHEN importance_score IS NULL THEN 1 ELSE 0 END) as unscored_messages,
                    COUNT(DISTINCT CASE WHEN importance_score IS NULL THEN session_id END) as sessions_with_unscored
                FROM messages
                "#,
            )
            .fetch_one(&self.pool)
            .await
            .map_err(|e| format!("Query failed: {}", e))?;

            Ok(ImportanceStats {
                total_messages: row.total_messages.unwrap_or(0),
                scored_messages: row.scored_messages.unwrap_or(0),
                unscored_messages: row.unscored_messages.unwrap_or(0),
                sessions_with_unscored: row.sessions_with_unscored.unwrap_or(0),
            })
        })
    }
}

impl Default for DbClient {
    fn default() -> Self {
        Self::new().expect("Failed to create database client")
    }
}
