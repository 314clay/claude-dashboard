//! HTTP client for the dashboard API.

use crate::graph::types::{GraphData, GraphEdge, GraphNode, PartialSummaryData, SessionSummaryData};
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
}

impl Default for ApiClient {
    fn default() -> Self {
        Self::new()
    }
}
