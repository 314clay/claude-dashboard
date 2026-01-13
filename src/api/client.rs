//! HTTP client for the dashboard API.

use crate::graph::types::{GraphData, GraphEdge, GraphNode, PartialSummaryData, SemanticFilter, SessionSummaryData};
use reqwest::blocking::Client;
use serde::Deserialize;
use std::time::Duration;

const API_BASE: &str = "http://127.0.0.1:8000";

#[derive(Debug, Deserialize)]
struct GraphResponse {
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    node_count: usize,
    edge_count: usize,
}

#[derive(Debug, Deserialize)]
struct HealthResponse {
    status: String,
}

/// Importance scoring statistics from the API
#[derive(Debug, Clone, Deserialize)]
pub struct ImportanceStats {
    pub total_messages: i64,
    pub scored_messages: i64,
    pub unscored_messages: i64,
    pub sessions_with_unscored: i64,
}

/// Result from rescore operation
#[derive(Debug, Clone, Deserialize)]
pub struct RescoreResult {
    pub sessions_processed: i64,
    pub messages_rescored: i64,
    pub errors: Vec<String>,
}

/// Progress update during rescore operation
#[derive(Debug, Clone)]
pub struct RescoreProgress {
    pub current: i64,
    pub total: i64,
    pub session_id: String,
    pub messages_so_far: i64,
}

/// Event from rescore stream
#[derive(Debug, Clone)]
pub enum RescoreEvent {
    Progress(RescoreProgress),
    Complete(RescoreResult),
    Error(String),
}

pub struct ApiClient {
    client: Client,
    base_url: String,
}

impl ApiClient {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            base_url: API_BASE.to_string(),
        }
    }

    /// Check if the API is healthy
    pub fn health(&self) -> Result<bool, String> {
        let url = format!("{}/health", self.base_url);
        match self.client.get(&url).send() {
            Ok(resp) => {
                if resp.status().is_success() {
                    Ok(true)
                } else {
                    Err(format!("API returned status: {}", resp.status()))
                }
            }
            Err(e) => Err(format!("Failed to connect to API: {}", e)),
        }
    }

    /// Fetch graph data from the API
    pub fn fetch_graph(&self, hours: f32, session_id: Option<&str>) -> Result<GraphData, String> {
        let mut url = format!("{}/graph?hours={}", self.base_url, hours);
        if let Some(sid) = session_id {
            url.push_str(&format!("&session_id={}", sid));
        }

        let resp = self
            .client
            .get(&url)
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("API error: {}", resp.status()));
        }

        let graph_resp: GraphResponse = resp
            .json()
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        Ok(GraphData {
            nodes: graph_resp.nodes,
            edges: graph_resp.edges,
        })
    }

    /// Fetch partial summary for a session up to a specific timestamp
    pub fn fetch_partial_summary(
        &self,
        session_id: &str,
        before_timestamp: &str,
    ) -> Result<PartialSummaryData, String> {
        let encoded_ts = urlencoding::encode(before_timestamp);
        let url = format!(
            "{}/session/{}/summary/partial?before_timestamp={}",
            self.base_url, session_id, encoded_ts
        );

        let resp = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(30)) // Longer timeout for AI generation
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("API error: {}", resp.status()));
        }

        let summary: PartialSummaryData = resp
            .json()
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        // Check for API-level errors
        if let Some(ref err) = summary.error {
            return Err(err.clone());
        }

        Ok(summary)
    }

    /// Fetch full session summary from the database, optionally generating if missing
    pub fn fetch_session_summary(
        &self,
        session_id: &str,
        generate_if_missing: bool,
    ) -> Result<SessionSummaryData, String> {
        let url = format!(
            "{}/session/{}/summary?generate={}",
            self.base_url, session_id, generate_if_missing
        );

        // Longer timeout if we might generate via AI
        let timeout = if generate_if_missing { 30 } else { 5 };

        let resp = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(timeout))
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("API error: {}", resp.status()));
        }

        let summary: SessionSummaryData = resp
            .json()
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        Ok(summary)
    }

    /// Fetch importance scoring statistics
    pub fn fetch_importance_stats(&self) -> Result<ImportanceStats, String> {
        let url = format!("{}/importance/stats", self.base_url);

        let resp = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("API error: {}", resp.status()));
        }

        let stats: ImportanceStats = resp
            .json()
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        Ok(stats)
    }

    /// Rescore importance for messages in specified sessions
    ///
    /// Overwrites existing scores atomically - if cancelled midway,
    /// no messages are left with NULL scores.
    pub fn rescore_importance(&self, session_ids: Vec<String>) -> Result<RescoreResult, String> {
        let url = format!("{}/importance/rescore", self.base_url);

        let resp = self
            .client
            .post(&url)
            .json(&serde_json::json!({ "session_ids": session_ids }))
            .timeout(Duration::from_secs(120)) // Long timeout for LLM calls
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("API error: {}", resp.status()));
        }

        let result: RescoreResult = resp
            .json()
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        Ok(result)
    }

    /// Rescore importance with streaming progress updates
    ///
    /// Calls the streaming endpoint and sends progress events through the channel.
    /// Returns when the stream completes or errors.
    pub fn rescore_importance_stream(
        &self,
        session_ids: Vec<String>,
        tx: std::sync::mpsc::Sender<RescoreEvent>,
    ) -> Result<(), String> {
        let url = format!("{}/importance/rescore/stream", self.base_url);

        let resp = self
            .client
            .post(&url)
            .json(&serde_json::json!({ "session_ids": session_ids }))
            .timeout(Duration::from_secs(600)) // Long timeout for LLM calls
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("API error: {}", resp.status()));
        }

        // Read SSE stream line by line
        use std::io::BufRead;
        let reader = std::io::BufReader::new(resp);

        for line in reader.lines() {
            let line = line.map_err(|e| format!("Read error: {}", e))?;

            // SSE format: "data: {...json...}"
            if let Some(json_str) = line.strip_prefix("data: ") {
                // Parse the JSON
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str) {
                    let event_type = value.get("type").and_then(|t| t.as_str());

                    match event_type {
                        Some("progress") => {
                            let progress = RescoreProgress {
                                current: value.get("current").and_then(|v| v.as_i64()).unwrap_or(0),
                                total: value.get("total").and_then(|v| v.as_i64()).unwrap_or(0),
                                session_id: value
                                    .get("session_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                messages_so_far: value
                                    .get("messages_so_far")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0),
                            };
                            let _ = tx.send(RescoreEvent::Progress(progress));
                        }
                        Some("complete") => {
                            let result = RescoreResult {
                                sessions_processed: value
                                    .get("sessions_processed")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0),
                                messages_rescored: value
                                    .get("messages_rescored")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0),
                                errors: value
                                    .get("errors")
                                    .and_then(|v| v.as_array())
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|v| v.as_str().map(String::from))
                                            .collect()
                                    })
                                    .unwrap_or_default(),
                            };
                            let _ = tx.send(RescoreEvent::Complete(result));
                            return Ok(());
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(())
    }

    /// Fetch all semantic filters
    pub fn fetch_semantic_filters(&self) -> Result<Vec<SemanticFilter>, String> {
        let url = format!("{}/semantic-filters", self.base_url);

        let resp = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("API error: {}", resp.status()));
        }

        #[derive(Deserialize)]
        struct FiltersResponse {
            filters: Vec<SemanticFilter>,
        }

        let response: FiltersResponse = resp
            .json()
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        Ok(response.filters)
    }

    /// Create a new semantic filter
    pub fn create_semantic_filter(&self, name: &str) -> Result<SemanticFilter, String> {
        let url = format!("{}/semantic-filters", self.base_url);

        let resp = self
            .client
            .post(&url)
            .json(&serde_json::json!({
                "name": name,
                "query_text": name
            }))
            .timeout(Duration::from_secs(5))
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("API error: {}", resp.status()));
        }

        let filter: SemanticFilter = resp
            .json()
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        Ok(filter)
    }

    /// Delete a semantic filter
    pub fn delete_semantic_filter(&self, id: i32) -> Result<(), String> {
        let url = format!("{}/semantic-filters/{}", self.base_url, id);

        let resp = self
            .client
            .delete(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("API error: {}", resp.status()));
        }

        Ok(())
    }

    /// Trigger categorization for a semantic filter
    pub fn trigger_categorization(&self, filter_id: i32) -> Result<(), String> {
        let url = format!("{}/semantic-filters/{}/categorize", self.base_url, filter_id);

        let resp = self
            .client
            .post(&url)
            .timeout(Duration::from_secs(60)) // Longer timeout for AI categorization
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("API error: {}", resp.status()));
        }

        Ok(())
    }
}

impl Default for ApiClient {
    fn default() -> Self {
        Self::new()
    }
}
