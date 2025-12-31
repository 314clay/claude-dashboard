//! HTTP client for the dashboard API.

use crate::graph::types::{GraphData, GraphEdge, GraphNode};
use reqwest::blocking::Client;
use serde::Deserialize;

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
}

impl Default for ApiClient {
    fn default() -> Self {
        Self::new()
    }
}
