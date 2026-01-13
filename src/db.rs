//! Direct PostgreSQL database access using sqlx.
//!
//! Replaces the Python API for core queries.

use chrono::{DateTime, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::{FromRow, PgPool};
use std::sync::Arc;
use tokio::runtime::Runtime;

use crate::graph::types::{GraphData, GraphEdge, GraphNode, Role, SessionSummaryData};

/// Database connection configuration
const DB_HOST: &str = "localhost";
const DB_PORT: u16 = 5433;
const DB_USER: &str = "clayarnold";
const DB_NAME: &str = "connectingservices";

/// Row returned from the graph query
#[derive(Debug, FromRow)]
struct MessageRow {
    id: i32,
    session_id: String,
    role: String,
    content: Option<String>,
    timestamp: Option<DateTime<Utc>>,
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
    topics: Option<serde_json::Value>,
    detected_project: Option<String>,
    generated_at: Option<DateTime<Utc>>,
}

/// Row returned from semantic filter matches query
#[derive(Debug, FromRow)]
struct FilterMatchRow {
    message_id: i64,  // bigint in DB
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
    pool: PgPool,
    runtime: Arc<Runtime>,
}

impl DbClient {
    /// Create a new database client
    pub fn new() -> Result<Self, String> {
        let runtime = Runtime::new().map_err(|e| format!("Failed to create runtime: {}", e))?;

        let pool = runtime.block_on(async {
            let url = format!(
                "postgres://{}@{}:{}/{}",
                DB_USER, DB_HOST, DB_PORT, DB_NAME
            );
            PgPoolOptions::new()
                .max_connections(5)
                .connect(&url)
                .await
        }).map_err(|e| format!("Failed to connect to database: {}", e))?;

        tracing::info!("Connected to PostgreSQL at {}:{}", DB_HOST, DB_PORT);

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
                    FROM claude_sessions.messages m
                    JOIN claude_sessions.sessions s ON m.session_id = s.session_id
                    WHERE m.session_id = $1
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
                    FROM claude_sessions.messages m
                    JOIN claude_sessions.sessions s ON m.session_id = s.session_id
                    WHERE m.timestamp >= NOW() - INTERVAL '1 hour' * $1
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
                let project = cwd.replace("/Users/clayarnold/", "~/");

                nodes.push(GraphNode {
                    id: msg_id.clone(),
                    role,
                    content_preview,
                    full_content: Some(content),
                    session_id: session_id.clone(),
                    session_short: session_id[..8.min(session_id.len())].to_string(),
                    project,
                    timestamp: row.timestamp.map(|t| t.to_rfc3339()),
                    importance_score: row.importance_score.map(|v| v as f32),
                    importance_reason: row.importance_reason,
                    output_tokens: row.token_count,
                    input_tokens: row.input_tokens,
                    cache_read_tokens: row.cache_read_tokens,
                    cache_creation_tokens: row.cache_creation_tokens,
                    semantic_filter_matches: Vec::new(), // Populated below
                });

                // Create edge from previous message in same session
                if let Some(prev_id) = prev_msg.get(&session_id) {
                    edges.push(GraphEdge {
                        source: prev_id.clone(),
                        target: msg_id.clone(),
                        session_id: session_id.clone(),
                        timestamp: row.timestamp.map(|t| t.to_rfc3339()),
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
                    let filter_matches: Vec<FilterMatchRow> = sqlx::query_as(
                        r#"
                        SELECT
                            r.message_id,
                            r.filter_id
                        FROM claude_sessions.semantic_filter_results r
                        JOIN claude_sessions.semantic_filters f ON r.filter_id = f.id
                        WHERE r.message_id = ANY($1)
                          AND r.matches = true
                          AND f.is_active = true
                        "#,
                    )
                    .bind(&message_ids)
                    .fetch_all(&self.pool)
                    .await
                    .unwrap_or_default();

                    // Build message_id -> filter_ids mapping
                    let mut matches_map: std::collections::HashMap<i32, Vec<i32>> = std::collections::HashMap::new();
                    for row in filter_matches {
                        // message_id is i64 in DB but i32 in nodes, convert safely
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

            Ok(GraphData { nodes, edges })
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
                FROM claude_sessions.session_summaries
                WHERE session_id = $1
                "#,
            )
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| format!("Query failed: {}", e))?;

            match row {
                Some(r) => {
                    let topics: Vec<String> = r.topics
                        .and_then(|v| serde_json::from_value(v).ok())
                        .unwrap_or_default();

                    Ok(SessionSummaryData {
                        exists: true,
                        generated: false,
                        summary: r.summary,
                        user_requests: r.user_requests,
                        completed_work: r.completed_work,
                        topics: Some(topics),
                        detected_project: r.detected_project,
                        generated_at: r.generated_at.map(|t| t.to_rfc3339()),
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
                    COUNT(*) FILTER (WHERE importance_score IS NOT NULL) as scored_messages,
                    COUNT(*) FILTER (WHERE importance_score IS NULL) as unscored_messages,
                    COUNT(DISTINCT session_id) FILTER (WHERE importance_score IS NULL) as sessions_with_unscored
                FROM claude_sessions.messages
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
