//! HTTP client for the dashboard API.

use crate::graph::types::{GraphData, GraphEdge, GraphNode, NeighborhoodSummaryData, PartialSummaryData, SemanticFilter, SessionSummaryData};
use std::collections::HashMap;
use crate::mail::MailNetworkData;
use reqwest::blocking::Client;
use serde::Deserialize;
use std::time::Duration;

fn api_base() -> String {
    std::env::var("API_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8000".to_string())
}

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

/// Embedding coverage statistics from the API
#[derive(Debug, Clone, Deserialize)]
pub struct EmbeddingStats {
    pub total: i64,
    pub embedded: i64,
    pub unembedded: i64,
    pub model: Option<String>,
}

/// Result from embedding generation
#[derive(Debug, Clone, Deserialize)]
pub struct EmbeddingGenResult {
    pub generated: i64,
    pub model: Option<String>,
    pub dimensions: Option<i64>,
    #[serde(default)]
    pub errors: Option<Vec<String>>,
    #[serde(default)]
    pub error: Option<String>,
}

/// A single proximity edge from the API
#[derive(Debug, Clone, Deserialize)]
pub struct ProximityEdgeResponse {
    pub source: String,
    pub target: String,
    pub strength: f32,
}

/// Response wrapper for proximity edges endpoint
#[derive(Debug, Clone, Deserialize)]
pub struct ProximityEdgesResponse {
    pub edges: Vec<ProximityEdgeResponse>,
    pub scores: HashMap<String, f32>,
    pub count: usize,
    pub query: String,
}

/// Progress for semantic filter categorization
#[derive(Debug, Clone, Deserialize)]
pub struct FilterStatusResponse {
    pub filter_id: i32,
    pub total: i64,
    pub scored: i64,
    pub pending: i64,
    pub matches: i64,
}

/// Result from session ingest operation
#[derive(Debug, Clone, Deserialize)]
pub struct IngestResult {
    pub sessions: i64,
    pub messages: i64,
    pub tools: i64,
    pub error: Option<String>,
}

pub struct ApiClient {
    client: Client,
    base_url: String,
}

impl ApiClient {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            base_url: api_base(),
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

    /// Trigger re-ingestion of Claude sessions
    pub fn trigger_ingest(&self, since: &str) -> Result<IngestResult, String> {
        let url = format!("{}/ingest?since={}", self.base_url, since);

        let resp = self
            .client
            .post(&url)
            .timeout(Duration::from_secs(300))
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("API error: {}", resp.status()));
        }

        let result: IngestResult = resp
            .json()
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        if let Some(ref err) = result.error {
            return Err(err.clone());
        }

        Ok(result)
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
            beads: Vec::new(),
            mail: Vec::new(),
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

    /// Fetch neighborhood summary for a cluster of graph-adjacent nodes
    pub fn fetch_neighborhood_summary(
        &self,
        message_ids: Vec<String>,
    ) -> Result<NeighborhoodSummaryData, String> {
        let url = format!("{}/summary/neighborhood", self.base_url);

        let resp = self
            .client
            .post(&url)
            .json(&serde_json::json!({ "message_ids": message_ids }))
            .timeout(Duration::from_secs(45)) // LLM generation can be slow
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("API error: {}", resp.status()));
        }

        let data: NeighborhoodSummaryData = resp
            .json()
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        if let Some(ref err) = data.error {
            return Err(err.clone());
        }

        Ok(data)
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

    /// Fetch mail network graph data for agent communication visualization.
    pub fn fetch_mail_network(&self) -> Result<MailNetworkData, String> {
        let url = format!("{}/mail/network", self.base_url);

        let resp = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("API error: {}", resp.status()));
        }

        let data: MailNetworkData = resp
            .json()
            .map_err(|e| format!("Failed to parse mail network: {}", e))?;

        Ok(data)
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
    pub fn create_semantic_filter(&self, name: &str, query_text: &str, filter_type: &str) -> Result<SemanticFilter, String> {
        let url = format!("{}/semantic-filters", self.base_url);

        let resp = self
            .client
            .post(&url)
            .json(&serde_json::json!({
                "name": name,
                "query_text": query_text,
                "filter_type": filter_type
            }))
            .timeout(Duration::from_secs(5))
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("API error: {}", resp.status()));
        }

        #[derive(Deserialize)]
        struct CreateResponse {
            success: bool,
            filter: Option<SemanticFilter>,
            error: Option<String>,
        }

        let response: CreateResponse = resp
            .json()
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        if !response.success {
            return Err(response.error.unwrap_or_else(|| "Unknown error".to_string()));
        }

        response.filter.ok_or_else(|| "No filter in response".to_string())
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
            .timeout(Duration::from_secs(300)) // Longer timeout for AI categorization
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("API error: {}", resp.status()));
        }

        Ok(())
    }

    /// Trigger categorization for only specific message IDs
    pub fn trigger_categorization_visible(&self, filter_id: i32, message_ids: &[String]) -> Result<(), String> {
        let url = format!("{}/semantic-filters/{}/categorize-visible", self.base_url, filter_id);

        let ids: Vec<i64> = message_ids.iter()
            .filter_map(|id| id.parse::<i64>().ok())
            .collect();

        let resp = self
            .client
            .post(&url)
            .json(&serde_json::json!({ "message_ids": ids }))
            .timeout(Duration::from_secs(120))
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("API error: {}", resp.status()));
        }

        Ok(())
    }

    /// Fetch filter scoring progress (for polling during categorization)
    pub fn fetch_filter_status(&self, filter_id: i32) -> Result<FilterStatusResponse, String> {
        let url = format!("{}/semantic-filters/{}/status", self.base_url, filter_id);

        let resp = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("API error: {}", resp.status()));
        }

        resp.json::<FilterStatusResponse>()
            .map_err(|e| format!("Parse error: {}", e))
    }

    /// Fetch embedding coverage statistics
    pub fn fetch_embedding_stats(&self) -> Result<EmbeddingStats, String> {
        let url = format!("{}/embeddings/stats", self.base_url);

        let resp = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("API error: {}", resp.status()));
        }

        let stats: EmbeddingStats = resp
            .json()
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        Ok(stats)
    }

    /// Generate embeddings for unembedded messages
    pub fn generate_embeddings(&self, max_messages: i32) -> Result<EmbeddingGenResult, String> {
        let url = format!(
            "{}/embeddings/generate?max_messages={}",
            self.base_url, max_messages
        );

        let resp = self
            .client
            .post(&url)
            .timeout(Duration::from_secs(120)) // Long timeout for embedding generation
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("API error: {}", resp.status()));
        }

        let result: EmbeddingGenResult = resp
            .json()
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        Ok(result)
    }

    /// Generate embeddings for specific message IDs only
    pub fn generate_embeddings_visible(&self, message_ids: &[String]) -> Result<EmbeddingGenResult, String> {
        let url = format!("{}/embeddings/generate-visible", self.base_url);

        let ids: Vec<i64> = message_ids.iter()
            .filter_map(|id| id.parse::<i64>().ok())
            .collect();

        let resp = self
            .client
            .post(&url)
            .json(&serde_json::json!({
                "message_ids": ids,
                "batch_size": 100,
                "max_messages": 50000,
            }))
            .timeout(Duration::from_secs(300))
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("API error: {}", resp.status()));
        }

        let result: EmbeddingGenResult = resp
            .json()
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        Ok(result)
    }

    /// Fetch proximity edges and scores from the API
    pub fn fetch_proximity_edges(
        &self,
        query_text: &str,
        delta: f32,
        max_edges: usize,
        max_neighbors: usize,
    ) -> Result<ProximityEdgesResponse, String> {
        let url = format!("{}/embeddings/proximity-edges", self.base_url);

        let resp = self
            .client
            .post(&url)
            .json(&serde_json::json!({
                "query_text": query_text,
                "delta": delta,
                "max_edges": max_edges,
                "max_neighbors": max_neighbors,
            }))
            .timeout(Duration::from_secs(60))
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("API error: {}", resp.status()));
        }

        let response: ProximityEdgesResponse = resp
            .json()
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        Ok(response)
    }

    /// Compute the visible set of message IDs based on semantic filter modes.
    /// Returns Ok(None) when no filters are active (all nodes visible).
    /// Returns Ok(Some(Vec<i64>)) with the visible message IDs when filters are active.
    pub fn compute_visible_set(&self, filter_modes: &HashMap<i32, String>, hours: f32) -> Result<Option<Vec<i64>>, String> {
        let url = format!("{}/filter/compute-visible", self.base_url);

        let body = serde_json::json!({
            "filter_modes": filter_modes,
            "hours": hours
        });

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .timeout(Duration::from_secs(30))
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("API error: {}", resp.status()));
        }

        #[derive(Deserialize)]
        struct ComputeVisibleResponse {
            visible_message_ids: Option<Vec<i64>>,
        }

        let response: ComputeVisibleResponse = resp
            .json()
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        Ok(response.visible_message_ids)
    }
}

impl Default for ApiClient {
    fn default() -> Self {
        Self::new()
    }
}
