//! Graph data types matching the API response.

use egui::Pos2;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

/// Timeline spacing mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimelineSpacingMode {
    #[default]
    TimeBased,
    EvenSpacing,
}

/// Role filter for hiding specific message types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum RoleFilter {
    #[default]
    ShowAll,
    HideClaude,
    HideUser,
}

impl RoleFilter {
    pub fn label(&self) -> &'static str {
        match self {
            RoleFilter::ShowAll => "Show All",
            RoleFilter::HideClaude => "Hide Claude",
            RoleFilter::HideUser => "Hide You",
        }
    }

    /// Check if a role should be visible under this filter
    pub fn is_visible(&self, role: &Role) -> bool {
        match self {
            RoleFilter::ShowAll => true,
            RoleFilter::HideClaude => *role != Role::Assistant,
            RoleFilter::HideUser => *role != Role::User,
        }
    }
}

/// Role of a message in the conversation
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    Obsidian,
    Topic,
}

impl Role {
    pub fn color(&self) -> egui::Color32 {
        match self {
            Role::User => egui::Color32::WHITE,
            Role::Assistant => egui::Color32::from_rgb(255, 149, 0), // Orange
            Role::Obsidian => egui::Color32::from_rgb(155, 89, 182), // Purple
            Role::Topic => egui::Color32::from_rgb(34, 197, 94),     // Green
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Role::User => "You",
            Role::Assistant => "Claude",
            Role::Obsidian => "Note",
            Role::Topic => "Topic",
        }
    }
}

/// A node in the conversation graph
#[derive(Debug, Clone, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub role: Role,
    pub content_preview: String,
    pub full_content: Option<String>,
    pub session_id: String,
    pub session_short: String,
    pub project: String,
    pub timestamp: Option<String>,
    #[serde(default)]
    pub importance_score: Option<f32>,
    #[serde(default)]
    pub importance_reason: Option<String>,
}

impl GraphNode {
    /// Parse timestamp string to epoch seconds
    pub fn timestamp_secs(&self) -> Option<f64> {
        self.timestamp.as_ref().and_then(|ts| {
            // Parse ISO 8601 format: "2025-12-31T01:30:07.726213+00:00"
            // Simple parsing - extract the key parts
            let ts = ts.replace('T', " ").replace('Z', "+00:00");

            // Try to parse with chrono-like manual parsing
            if let Some(plus_idx) = ts.rfind('+') {
                let datetime_part = &ts[..plus_idx];
                // Parse: "2025-12-31 01:30:07.726213"
                let parts: Vec<&str> = datetime_part.split(' ').collect();
                if parts.len() >= 2 {
                    let date_parts: Vec<&str> = parts[0].split('-').collect();
                    let time_full = parts[1];
                    let time_parts: Vec<&str> = time_full.split(':').collect();

                    if date_parts.len() >= 3 && time_parts.len() >= 3 {
                        let year: i32 = date_parts[0].parse().ok()?;
                        let month: u32 = date_parts[1].parse().ok()?;
                        let day: u32 = date_parts[2].parse().ok()?;
                        let hour: u32 = time_parts[0].parse().ok()?;
                        let min: u32 = time_parts[1].parse().ok()?;
                        let sec_str = time_parts[2].split('.').next()?;
                        let sec: u32 = sec_str.parse().ok()?;

                        // Simple epoch calculation (approximate, ignores leap seconds)
                        let days_since_epoch = days_from_civil(year, month, day);
                        let secs = days_since_epoch as f64 * 86400.0
                            + hour as f64 * 3600.0
                            + min as f64 * 60.0
                            + sec as f64;
                        return Some(secs);
                    }
                }
            }
            None
        })
    }
}

/// Calculate days since Unix epoch (simple implementation)
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let m = month;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

/// An edge connecting two nodes
#[derive(Debug, Clone, Deserialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    pub session_id: String,
    pub timestamp: Option<String>,
    #[serde(default)]
    pub is_obsidian: bool,
    #[serde(default)]
    pub is_topic: bool,
    #[serde(default)]
    pub is_similarity: bool,
    #[serde(default)]
    pub is_temporal: bool,
    /// Strength multiplier for this edge (used by temporal and similarity edges)
    pub similarity: Option<f32>,
}

impl GraphEdge {
    /// Create a temporal edge between two nodes
    pub fn temporal(source: String, target: String, strength: f32) -> Self {
        Self {
            source,
            target,
            session_id: String::new(),
            timestamp: None,
            is_obsidian: false,
            is_topic: false,
            is_similarity: false,
            is_temporal: true,
            similarity: Some(strength),
        }
    }
}

/// Complete graph data from the API
#[derive(Debug, Clone, Default)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

/// Partial summary data from the API (generated by Gemini)
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PartialSummaryData {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub completed_work: String,
    #[serde(default)]
    pub unsuccessful_attempts: String,
    #[serde(default)]
    pub current_focus: String,
    #[serde(default)]
    pub user_count: u32,
    #[serde(default)]
    pub assistant_count: u32,
    #[serde(default)]
    pub error: Option<String>,
}

/// Timeline state for scrubbing through time
#[derive(Debug, Clone)]
pub struct TimelineState {
    /// Sorted node indices by timestamp
    pub sorted_indices: Vec<usize>,
    /// Timestamps in seconds for each sorted node
    pub timestamps: Vec<f64>,
    /// Min timestamp in the data
    pub min_time: f64,
    /// Max timestamp in the data
    pub max_time: f64,
    /// Current scrubber position (0.0 - 1.0)
    pub position: f32,
    /// Start position for time window (0.0 - 1.0)
    pub start_position: f32,
    /// Is playback active?
    pub playing: bool,
    /// Playback speed multiplier
    pub speed: f32,
    /// Set of visible node IDs based on current time window
    pub visible_nodes: HashSet<String>,
    /// Spacing mode for timeline display
    pub spacing_mode: TimelineSpacingMode,
}

impl Default for TimelineState {
    fn default() -> Self {
        Self {
            sorted_indices: Vec::new(),
            timestamps: Vec::new(),
            min_time: 0.0,
            max_time: 0.0,
            position: 1.0,
            start_position: 0.0,
            playing: false,
            speed: 1.0,
            visible_nodes: HashSet::new(),
            spacing_mode: TimelineSpacingMode::default(),
        }
    }
}

impl TimelineState {
    /// Get time at a given position (0.0 - 1.0)
    pub fn time_at_position(&self, pos: f32) -> f64 {
        self.min_time + (self.max_time - self.min_time) * pos as f64
    }

    /// Get position for a given time
    pub fn position_at_time(&self, time: f64) -> f32 {
        if self.max_time <= self.min_time {
            return 1.0;
        }
        ((time - self.min_time) / (self.max_time - self.min_time)) as f32
    }

    /// Format a time as a human-readable string
    pub fn format_time(&self, time: f64) -> String {
        // Convert epoch seconds back to readable format
        let total_secs = time as i64;
        let days = total_secs / 86400;
        let hours = (total_secs % 86400) / 3600;
        let mins = (total_secs % 3600) / 60;

        // Calculate date from days since epoch
        let (year, month, day) = civil_from_days(days);
        format!("{:04}-{:02}-{:02} {:02}:{:02}", year, month, day, hours, mins)
    }

    /// Get the index of the nearest notch (node) to a position
    pub fn nearest_notch(&self, pos: f32) -> Option<usize> {
        if self.timestamps.is_empty() {
            return None;
        }
        let target_time = self.time_at_position(pos);
        let mut best_idx = 0;
        let mut best_diff = f64::MAX;
        for (i, &t) in self.timestamps.iter().enumerate() {
            let diff = (t - target_time).abs();
            if diff < best_diff {
                best_diff = diff;
                best_idx = i;
            }
        }
        Some(best_idx)
    }

    /// Snap position to nearest notch
    pub fn snap_to_notch(&self, pos: f32) -> f32 {
        if let Some(idx) = self.nearest_notch(pos) {
            if idx < self.timestamps.len() {
                return self.position_at_time(self.timestamps[idx]);
            }
        }
        pos
    }

    /// Get position for a node at the given sorted index, respecting spacing mode
    pub fn position_for_index(&self, index: usize) -> f32 {
        match self.spacing_mode {
            TimelineSpacingMode::TimeBased => {
                if index < self.timestamps.len() {
                    self.position_at_time(self.timestamps[index])
                } else {
                    1.0
                }
            }
            TimelineSpacingMode::EvenSpacing => {
                let count = self.timestamps.len();
                if count <= 1 {
                    1.0
                } else {
                    index as f32 / (count - 1) as f32
                }
            }
        }
    }

    /// Get the sorted index for a given position, respecting spacing mode
    pub fn index_at_position(&self, pos: f32) -> usize {
        let count = self.timestamps.len();
        if count == 0 {
            return 0;
        }

        match self.spacing_mode {
            TimelineSpacingMode::TimeBased => {
                // Find closest timestamp to the time at this position
                let target_time = self.time_at_position(pos);
                let mut best_idx = 0;
                let mut best_diff = f64::MAX;
                for (i, &t) in self.timestamps.iter().enumerate() {
                    let diff = (t - target_time).abs();
                    if diff < best_diff {
                        best_diff = diff;
                        best_idx = i;
                    }
                }
                best_idx
            }
            TimelineSpacingMode::EvenSpacing => {
                // Direct mapping: position * (count - 1)
                let index = (pos * (count - 1) as f32).round() as usize;
                index.min(count - 1)
            }
        }
    }

    /// Snap position to nearest notch, respecting spacing mode
    pub fn snap_to_notch_modal(&self, pos: f32) -> f32 {
        let idx = self.index_at_position(pos);
        self.position_for_index(idx)
    }
}

/// Convert days since epoch to civil date
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y } as i32;
    (year, m, d)
}

/// Runtime graph state with positions
pub struct GraphState {
    /// Node positions (id -> position)
    pub positions: HashMap<String, Pos2>,
    /// Node velocities for physics simulation
    pub velocities: HashMap<String, egui::Vec2>,
    /// The underlying data
    pub data: GraphData,
    /// Node index lookup (id -> index in data.nodes)
    pub node_index: HashMap<String, usize>,
    /// Session colors (session_id -> hue)
    pub session_colors: HashMap<String, f32>,
    /// Project colors (project_name -> hue)
    pub project_colors: HashMap<String, f32>,
    /// Color mode: true = by project, false = by session
    pub color_by_project: bool,
    /// Is physics simulation running?
    pub physics_enabled: bool,
    /// Currently hovered node
    pub hovered_node: Option<String>,
    /// Currently selected node
    pub selected_node: Option<String>,
    /// Timeline state
    pub timeline: TimelineState,
    /// Temporal attraction enabled
    pub temporal_attraction_enabled: bool,
    /// Temporal window in seconds (nodes within this window attract)
    pub temporal_window_secs: f64,
    /// Session chains: ordered node IDs per session (for importance bridging)
    pub session_chains: HashMap<String, Vec<String>>,
}

impl GraphState {
    pub fn new() -> Self {
        Self {
            positions: HashMap::new(),
            velocities: HashMap::new(),
            data: GraphData::default(),
            node_index: HashMap::new(),
            session_colors: HashMap::new(),
            project_colors: HashMap::new(),
            color_by_project: true, // Default to project coloring
            physics_enabled: true,
            hovered_node: None,
            selected_node: None,
            timeline: TimelineState::default(),
            temporal_attraction_enabled: true,
            temporal_window_secs: 300.0, // 5 minutes default
            session_chains: HashMap::new(),
        }
    }

    /// Load new graph data, initializing positions randomly
    pub fn load(&mut self, data: GraphData, bounds: egui::Rect) {
        use rand::Rng;
        let mut rng = rand::thread_rng();

        // Clear old state
        self.positions.clear();
        self.velocities.clear();
        self.node_index.clear();
        self.session_colors.clear();
        self.project_colors.clear();
        self.session_chains.clear();

        // Build node index and initialize positions
        for (i, node) in data.nodes.iter().enumerate() {
            self.node_index.insert(node.id.clone(), i);

            // Random initial position within bounds
            let x = rng.gen_range(bounds.min.x..bounds.max.x);
            let y = rng.gen_range(bounds.min.y..bounds.max.y);
            self.positions.insert(node.id.clone(), Pos2::new(x, y));
            self.velocities.insert(node.id.clone(), egui::Vec2::ZERO);

            // Assign session color if not already assigned
            if !self.session_colors.contains_key(&node.session_id) {
                let hue = (self.session_colors.len() as f32 * 137.5) % 360.0;
                self.session_colors.insert(node.session_id.clone(), hue);
            }

            // Assign project color if not already assigned
            if !node.project.is_empty() && !self.project_colors.contains_key(&node.project) {
                // Use golden ratio for better color distribution
                let hue = (self.project_colors.len() as f32 * 137.5) % 360.0;
                self.project_colors.insert(node.project.clone(), hue);
            }
        }

        self.data = data;
        self.physics_enabled = true;

        // Build session chains from edges (for importance bridging)
        self.build_session_chains();

        // Build timeline data
        self.build_timeline();

        // Build temporal edges (pre-computed at load time)
        if self.temporal_attraction_enabled {
            self.build_temporal_edges();
        }
    }

    /// Maximum number of temporal edges to prevent performance issues with large datasets.
    /// Beyond this limit, temporal edges are sampled to stay within budget.
    const MAX_TEMPORAL_EDGES: usize = 10_000;

    /// Node count threshold above which temporal edges are auto-disabled.
    const TEMPORAL_EDGE_NODE_LIMIT: usize = 2000;

    /// Build pre-computed temporal edges between nodes close in time.
    /// Uses sliding window algorithm: O(n) instead of O(n²).
    ///
    /// Safety: Limits total temporal edges to MAX_TEMPORAL_EDGES to prevent
    /// memory and performance issues with large datasets.
    pub fn build_temporal_edges(&mut self) {
        // Remove any existing temporal edges first
        self.data.edges.retain(|e| !e.is_temporal);

        if self.timeline.sorted_indices.is_empty() {
            return;
        }

        // Auto-disable for very large datasets to prevent freezing
        let node_count = self.timeline.sorted_indices.len();
        if node_count > Self::TEMPORAL_EDGE_NODE_LIMIT {
            #[cfg(debug_assertions)]
            eprintln!(
                "Skipping temporal edges: {} nodes exceeds limit of {}",
                node_count,
                Self::TEMPORAL_EDGE_NODE_LIMIT
            );
            return;
        }

        let window = self.temporal_window_secs;
        let mut temporal_edges = Vec::new();

        // Sliding window over sorted timestamps
        // For each node, connect to all following nodes within the time window
        for i in 0..node_count {
            let node_i_idx = self.timeline.sorted_indices[i];
            let ts_i = self.timeline.timestamps[i];

            for j in (i + 1)..node_count {
                let ts_j = self.timeline.timestamps[j];
                let dt = ts_j - ts_i;

                // Since sorted, if we exceed window we're done with this node
                if dt > window {
                    break;
                }

                let node_j_idx = self.timeline.sorted_indices[j];

                // Strength decays linearly from 1.0 to 0.0 over the window
                let strength = 1.0 - (dt / window) as f32;

                let source_id = self.data.nodes[node_i_idx].id.clone();
                let target_id = self.data.nodes[node_j_idx].id.clone();

                temporal_edges.push(GraphEdge::temporal(source_id, target_id, strength));

                // Hard cap to prevent runaway memory/performance issues
                if temporal_edges.len() >= Self::MAX_TEMPORAL_EDGES {
                    #[cfg(debug_assertions)]
                    eprintln!(
                        "Hit temporal edge limit of {} (window: {}s)",
                        Self::MAX_TEMPORAL_EDGES,
                        window
                    );
                    self.data.edges.extend(temporal_edges);
                    return;
                }
            }
        }

        // Debug: temporal edges created
        #[cfg(debug_assertions)]
        eprintln!(
            "Built {} temporal edges (window: {}s, nodes: {})",
            temporal_edges.len(),
            window,
            node_count
        );

        self.data.edges.extend(temporal_edges);
    }

    /// Rebuild temporal edges with a new window size
    pub fn set_temporal_window(&mut self, window_secs: f64) {
        self.temporal_window_secs = window_secs;
        if self.temporal_attraction_enabled {
            self.build_temporal_edges();
        }
    }

    /// Toggle temporal attraction on/off
    pub fn set_temporal_attraction_enabled(&mut self, enabled: bool) {
        self.temporal_attraction_enabled = enabled;
        if enabled {
            self.build_temporal_edges();
        } else {
            // Remove temporal edges
            self.data.edges.retain(|e| !e.is_temporal);
        }
    }

    /// Build session chains: ordered node IDs per session.
    /// Used for "bridging" edges when nodes are filtered by importance.
    fn build_session_chains(&mut self) {
        self.session_chains.clear();

        // Build chains by traversing non-temporal edges (sequential session edges)
        // Each edge source→target means target comes after source in that session
        for edge in &self.data.edges {
            if edge.is_temporal || edge.is_similarity || edge.is_topic || edge.is_obsidian {
                continue; // Skip non-session edges
            }

            let chain = self
                .session_chains
                .entry(edge.session_id.clone())
                .or_insert_with(Vec::new);

            // If chain is empty, add source first
            if chain.is_empty() {
                chain.push(edge.source.clone());
            }
            chain.push(edge.target.clone());
        }
    }

    /// Get the next visible node in the session chain after the given node.
    /// Considers both importance threshold and role filter.
    /// Returns None if no next visible node exists.
    pub fn next_visible_in_chain(
        &self,
        node_id: &str,
        importance_threshold: f32,
        role_filter: RoleFilter,
    ) -> Option<String> {
        // Find which session this node belongs to
        let node = self.get_node(node_id)?;
        let chain = self.session_chains.get(&node.session_id)?;

        // Find position of current node in chain
        let pos = chain.iter().position(|id| id == node_id)?;

        // Look for next visible node (must pass both importance and role filter)
        for next_id in chain.iter().skip(pos + 1) {
            if let Some(next_node) = self.get_node(next_id) {
                let score = next_node.importance_score.unwrap_or(1.0);
                let passes_importance = score >= importance_threshold;
                let passes_role = role_filter.is_visible(&next_node.role);
                if passes_importance && passes_role {
                    return Some(next_id.clone());
                }
            }
        }
        None
    }

    /// Build timeline sorted indices and timestamps
    fn build_timeline(&mut self) {
        // Collect nodes with valid timestamps
        let mut timed_nodes: Vec<(usize, f64)> = self
            .data
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(i, node)| node.timestamp_secs().map(|t| (i, t)))
            .collect();

        // Sort by timestamp
        timed_nodes.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        // Extract sorted indices and timestamps
        self.timeline.sorted_indices = timed_nodes.iter().map(|(i, _)| *i).collect();
        self.timeline.timestamps = timed_nodes.iter().map(|(_, t)| *t).collect();

        // Set time range
        if let (Some(&first), Some(&last)) = (self.timeline.timestamps.first(), self.timeline.timestamps.last()) {
            self.timeline.min_time = first;
            self.timeline.max_time = last;
        }

        // Initialize with all nodes visible
        self.timeline.position = 1.0;
        self.timeline.start_position = 0.0;
        self.update_visible_nodes();
    }

    /// Update which nodes are visible based on timeline position
    pub fn update_visible_nodes(&mut self) {
        self.timeline.visible_nodes.clear();

        let count = self.timeline.sorted_indices.len();
        if count == 0 {
            return;
        }

        match self.timeline.spacing_mode {
            TimelineSpacingMode::TimeBased => {
                // Original time-based logic
                let start_time = self.timeline.time_at_position(self.timeline.start_position);
                let end_time = self.timeline.time_at_position(self.timeline.position);

                for (i, &idx) in self.timeline.sorted_indices.iter().enumerate() {
                    let t = self.timeline.timestamps[i];
                    if t >= start_time && t <= end_time {
                        if let Some(node) = self.data.nodes.get(idx) {
                            self.timeline.visible_nodes.insert(node.id.clone());
                        }
                    }
                }
            }
            TimelineSpacingMode::EvenSpacing => {
                // Index-based visibility
                let start_idx = self.timeline.index_at_position(self.timeline.start_position);
                let end_idx = self.timeline.index_at_position(self.timeline.position);

                for i in start_idx..=end_idx {
                    if i < self.timeline.sorted_indices.len() {
                        let node_idx = self.timeline.sorted_indices[i];
                        if let Some(node) = self.data.nodes.get(node_idx) {
                            self.timeline.visible_nodes.insert(node.id.clone());
                        }
                    }
                }
            }
        }
    }

    /// Check if a node is visible in the current timeline window
    pub fn is_node_visible(&self, id: &str) -> bool {
        self.timeline.visible_nodes.contains(id)
    }

    /// Check if an edge should be visible (both endpoints visible)
    pub fn is_edge_visible(&self, edge: &GraphEdge) -> bool {
        self.timeline.visible_nodes.contains(&edge.source)
            && self.timeline.visible_nodes.contains(&edge.target)
    }

    /// Get the position of a node
    pub fn get_pos(&self, id: &str) -> Option<Pos2> {
        self.positions.get(id).copied()
    }

    /// Get a node by ID
    pub fn get_node(&self, id: &str) -> Option<&GraphNode> {
        self.node_index.get(id).map(|&i| &self.data.nodes[i])
    }

    /// Get the color for a node based on current color mode
    pub fn node_color(&self, node: &GraphNode) -> egui::Color32 {
        if self.color_by_project && !node.project.is_empty() {
            let hue = self.project_colors.get(&node.project).copied().unwrap_or(0.0);
            hsl_to_rgb(hue, 0.7, 0.55)
        } else {
            let hue = self.session_colors.get(&node.session_id).copied().unwrap_or(0.0);
            hsl_to_rgb(hue, 0.7, 0.5)
        }
    }

    /// Get a lighter version of node color (for fills)
    pub fn node_color_light(&self, node: &GraphNode) -> egui::Color32 {
        if self.color_by_project && !node.project.is_empty() {
            let hue = self.project_colors.get(&node.project).copied().unwrap_or(0.0);
            hsl_to_rgb(hue, 0.6, 0.75)
        } else {
            let hue = self.session_colors.get(&node.session_id).copied().unwrap_or(0.0);
            hsl_to_rgb(hue, 0.6, 0.7)
        }
    }

    /// Get the session color (hue) for an edge
    pub fn edge_color(&self, edge: &GraphEdge) -> egui::Color32 {
        if edge.is_similarity {
            egui::Color32::from_rgb(6, 182, 212) // Cyan
        } else if edge.is_topic {
            egui::Color32::from_rgb(34, 197, 94) // Green
        } else if edge.is_obsidian {
            egui::Color32::from_rgb(155, 89, 182) // Purple
        } else if self.color_by_project {
            // Find source node's project for edge color
            if let Some(node) = self.get_node(&edge.source) {
                if !node.project.is_empty() {
                    let hue = self.project_colors.get(&node.project).copied().unwrap_or(0.0);
                    return hsl_to_rgb(hue, 0.5, 0.4);
                }
            }
            // Fallback to session color
            let hue = self.session_colors.get(&edge.session_id).copied().unwrap_or(0.0);
            hsl_to_rgb(hue, 0.5, 0.4)
        } else {
            // Session-based color
            let hue = self.session_colors.get(&edge.session_id).copied().unwrap_or(0.0);
            hsl_to_rgb(hue, 0.7, 0.5)
        }
    }
}

impl Default for GraphState {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert HSL to RGB color
pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> egui::Color32 {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;

    let (r, g, b) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    egui::Color32::from_rgb(
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}
