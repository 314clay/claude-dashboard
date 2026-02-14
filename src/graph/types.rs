//! Graph data types matching the API response.

use egui::Pos2;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

/// Mode for semantic filter application
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SemanticFilterMode {
    #[default]
    Off,          // Don't apply this filter
    Exclude,      // Hide nodes that MATCH
    Include,      // Only show nodes that MATCH
    IncludePlus1, // Show matching nodes + their direct neighbors (BFS depth 1)
    IncludePlus2, // Show matching nodes + neighbors up to depth 2
}

/// 3-way filter mode: Off / Inactive (dim, bypass edges) / Filtered (fully removed)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum FilterMode {
    #[default]
    Off,
    Inactive,  // Nodes hidden, bypass edges maintain connectivity
    Filtered,  // Nodes + all edges truly removed
}

impl FilterMode {
    pub fn is_active(&self) -> bool { *self != FilterMode::Off }
    pub fn label(&self) -> &'static str {
        match self { Self::Off => "Off", Self::Inactive => "Dim", Self::Filtered => "Hide" }
    }
}

/// Color mode for graph visualization
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ColorMode {
    #[default]
    Project,  // All sessions in same project share same hue
    Session,  // Each session gets its own hue
    Hybrid,   // Project hue + session S/L variation (temporally similar = similar shade)
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
    #[serde(default)]
    pub output_tokens: Option<i32>,
    #[serde(default)]
    pub input_tokens: Option<i32>,
    #[serde(default)]
    pub cache_read_tokens: Option<i32>,
    #[serde(default)]
    pub cache_creation_tokens: Option<i32>,
    #[serde(default)]
    pub semantic_filter_matches: Vec<i32>,
    #[serde(default)]
    pub has_tool_usage: bool,
}

impl GraphNode {
    /// Get total tokens (output + input) for sizing
    pub fn total_tokens(&self) -> i32 {
        self.output_tokens.unwrap_or(0) + self.input_tokens.unwrap_or(0)
    }

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
    /// Which proximity query produced this edge (for multi-query coloring)
    #[serde(default)]
    pub query_index: Option<usize>,
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
            query_index: None,
        }
    }

    /// Create a similarity edge between two nodes
    pub fn similarity(source: String, target: String, strength: f32, query_index: Option<usize>) -> Self {
        Self {
            source,
            target,
            session_id: String::new(),
            timestamp: None,
            is_obsidian: false,
            is_topic: false,
            is_similarity: true,
            is_temporal: false,
            similarity: Some(strength),
            query_index,
        }
    }
}

/// Issue status for Kanban columns
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum IssueStatus {
    #[default]
    Open,
    InProgress,
    Blocked,
    Closed,
    Deferred,
    Hooked,
}

impl IssueStatus {
    pub fn label(&self) -> &'static str {
        match self {
            IssueStatus::Open => "Open",
            IssueStatus::InProgress => "In Progress",
            IssueStatus::Blocked => "Blocked",
            IssueStatus::Closed => "Closed",
            IssueStatus::Deferred => "Deferred",
            IssueStatus::Hooked => "Hooked",
        }
    }
}

/// A bead (issue) item for display in panels
#[derive(Debug, Clone)]
pub struct BeadItem {
    pub id: String,
    pub title: String,
    pub status: IssueStatus,
    pub labels: Vec<String>,
    pub priority: i32,
    /// ISO 8601 timestamp when created
    pub created_at: Option<String>,
    /// ISO 8601 timestamp when last updated
    pub updated_at: Option<String>,
    pub issue_type: Option<String>,
    pub description: Option<String>,
    pub assignee: Option<String>,
}

impl BeadItem {
    /// Parse created_at timestamp to epoch seconds for timeline filtering
    pub fn timestamp_secs(&self) -> Option<f64> {
        self.created_at.as_ref().and_then(|ts| parse_iso_timestamp(ts))
    }

    /// Parse updated_at timestamp to epoch seconds
    pub fn updated_at_secs(&self) -> Option<f64> {
        self.updated_at.as_ref().and_then(|ts| parse_iso_timestamp(ts))
    }
}

/// A mail item for display in inbox/outbox panels
#[derive(Debug, Clone)]
pub struct MailItem {
    pub id: String,
    pub subject: String,
    pub sender: String,
    pub recipient: String,
    /// ISO 8601 timestamp when sent/received
    pub timestamp: Option<String>,
    /// Thread ID for grouping related messages
    pub thread_id: Option<String>,
    /// True if this message hasn't been read
    pub is_unread: bool,
    /// Preview of the message content
    pub preview: Option<String>,
}

impl MailItem {
    /// Parse timestamp to epoch seconds for timeline filtering
    pub fn timestamp_secs(&self) -> Option<f64> {
        self.timestamp.as_ref().and_then(|ts| parse_iso_timestamp(ts))
    }
}

/// Parse an ISO 8601 timestamp to epoch seconds
fn parse_iso_timestamp(ts: &str) -> Option<f64> {
    // Parse ISO 8601 format: "2025-12-31T01:30:07.726213+00:00" or "2025-12-31"
    let ts = ts.replace('T', " ").replace('Z', "+00:00");

    // Handle date-only format "2025-12-31"
    if !ts.contains(' ') {
        let date_parts: Vec<&str> = ts.split('-').collect();
        if date_parts.len() >= 3 {
            let year: i32 = date_parts[0].parse().ok()?;
            let month: u32 = date_parts[1].parse().ok()?;
            let day: u32 = date_parts[2].split('+').next()?.parse().ok()?;
            let days_since_epoch = days_from_civil(year, month, day);
            return Some(days_since_epoch as f64 * 86400.0);
        }
        return None;
    }

    // Try to parse datetime with timezone
    if let Some(plus_idx) = ts.rfind('+') {
        let datetime_part = &ts[..plus_idx];
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
}

/// Complete graph data from the API
#[derive(Debug, Clone, Default)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    /// Bead (issue) items for unified timeline filtering
    pub beads: Vec<BeadItem>,
    /// Mail items for unified timeline filtering
    pub mail: Vec<MailItem>,
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

/// Full session summary from the database (pre-generated or just-generated)
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SessionSummaryData {
    #[serde(default)]
    pub exists: bool,
    /// True if the summary was just generated (not from cache)
    #[serde(default)]
    pub generated: bool,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub user_requests: Option<String>,
    #[serde(default)]
    pub completed_work: Option<String>,
    #[serde(default)]
    pub topics: Option<Vec<String>>,
    #[serde(default)]
    pub detected_project: Option<String>,
    #[serde(default)]
    pub generated_at: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Neighborhood summary data from the API (cluster of adjacent nodes)
#[derive(Debug, Clone, Default, Deserialize)]
pub struct NeighborhoodSummaryData {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub themes: String,
    #[serde(default)]
    pub node_count: u32,
    #[serde(default)]
    pub session_count: u32,
    #[serde(default)]
    pub error: Option<String>,
}

fn default_filter_type() -> String {
    "semantic".to_string()
}

/// A semantic filter for categorizing messages
#[derive(Debug, Clone, Deserialize)]
pub struct SemanticFilter {
    pub id: i32,
    pub name: String,
    pub query_text: String,
    #[serde(default = "default_filter_type")]
    pub filter_type: String,
    pub is_active: bool,
    #[serde(default)]
    pub total_scored: i64,
    #[serde(default)]
    pub matches: i64,
}

impl SemanticFilter {
    /// Returns true if this is a rule-based (non-LLM) filter
    pub fn is_rule(&self) -> bool {
        self.filter_type == "rule"
    }
}

/// Timeline state for scrubbing through time
/// This provides unified timeline filtering across all panels:
/// - Graph nodes (visible_nodes)
/// - Beads/issues (visible_beads)
/// - Mail items (visible_mail)
#[derive(Debug, Clone)]
pub struct TimelineState {
    /// Sorted node indices by timestamp
    pub sorted_indices: Vec<usize>,
    /// Timestamps in seconds for each sorted node
    pub timestamps: Vec<f64>,
    /// Min timestamp in the data (considering all items: nodes, beads, mail)
    pub min_time: f64,
    /// Max timestamp in the data (considering all items: nodes, beads, mail)
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

    // --- Unified Timeline: Bead filtering ---
    /// Sorted bead indices by timestamp
    pub sorted_bead_indices: Vec<usize>,
    /// Timestamps in seconds for each sorted bead
    pub bead_timestamps: Vec<f64>,
    /// Set of visible bead IDs based on current time window
    pub visible_beads: HashSet<String>,

    // --- Unified Timeline: Mail filtering ---
    /// Sorted mail indices by timestamp
    pub sorted_mail_indices: Vec<usize>,
    /// Timestamps in seconds for each sorted mail item
    pub mail_timestamps: Vec<f64>,
    /// Set of visible mail IDs based on current time window
    pub visible_mail: HashSet<String>,
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
            // Bead filtering
            sorted_bead_indices: Vec::new(),
            bead_timestamps: Vec::new(),
            visible_beads: HashSet::new(),
            // Mail filtering
            sorted_mail_indices: Vec::new(),
            mail_timestamps: Vec::new(),
            visible_mail: HashSet::new(),
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
        // Get current time for relative formatting
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let timestamp = time as i64;
        let diff_secs = now - timestamp;

        // Relative time for recent events
        if diff_secs < 60 {
            return "Just now".to_string();
        } else if diff_secs < 3600 {
            let mins = diff_secs / 60;
            return format!("{} min{} ago", mins, if mins == 1 { "" } else { "s" });
        } else if diff_secs < 86400 {
            let hours = diff_secs / 3600;
            return format!("{} hour{} ago", hours, if hours == 1 { "" } else { "s" });
        }

        // Calculate date components
        let days = timestamp / 86400;
        let hours = (timestamp % 86400) / 3600;
        let mins = (timestamp % 3600) / 60;
        let (year, month, day) = civil_from_days(days);

        // Convert to 12-hour format
        let (hour_12, am_pm) = if hours == 0 {
            (12, "AM")
        } else if hours < 12 {
            (hours, "AM")
        } else if hours == 12 {
            (12, "PM")
        } else {
            (hours - 12, "PM")
        };

        // Format based on how old it is
        let now_days = now / 86400;
        let day_diff = now_days - days;

        if day_diff == 0 {
            format!("Today at {}:{:02} {}", hour_12, mins, am_pm)
        } else if day_diff == 1 {
            format!("Yesterday at {}:{:02} {}", hour_12, mins, am_pm)
        } else if day_diff < 7 {
            let weekday = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
            let day_of_week = ((days + 4) % 7) as usize; // Jan 1, 1970 was Thursday
            format!("{} at {}:{:02} {}", weekday[day_of_week], hour_12, mins, am_pm)
        } else {
            let month_name = ["Jan", "Feb", "Mar", "Apr", "May", "Jun",
                            "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
            let current_year = civil_from_days(now_days).0;
            if year == current_year {
                format!("{} {} at {}:{:02} {}", month_name[(month - 1) as usize], day, hour_12, mins, am_pm)
            } else {
                format!("{} {}, {} at {}:{:02} {}", month_name[(month - 1) as usize], day, year, hour_12, mins, am_pm)
            }
        }
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
    /// Tracks how many children we've seen for each parent path
    pub child_counts: HashMap<String, usize>,
    /// Maps "parent:child" to the child's sibling index
    pub child_indices: HashMap<String, usize>,
    /// Global hue offset for randomizing colors while preserving relationships
    pub hue_offset: f32,
    /// Color mode for graph visualization
    pub color_mode: ColorMode,
    /// Sessions within each project, sorted by timestamp: project -> [(session_id, timestamp)]
    /// Used for hybrid coloring to give temporally close sessions similar shades
    pub project_sessions: HashMap<String, Vec<(String, f64)>>,
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
    /// Maximum temporal edges to build
    pub max_temporal_edges: usize,
    /// Maximum total tokens across all nodes (for normalization)
    pub max_tokens: i32,
    /// Whether score-proximity edges are enabled
    pub score_proximity_enabled: bool,
    /// Maximum score difference to create a proximity edge
    pub score_proximity_delta: f32,
    /// Maximum proximity edges to prevent memory issues
    pub max_proximity_edges: usize,
    /// Per-node edge cap for proximity edges (0 = unlimited)
    pub max_neighbors_per_node: usize,
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
            child_counts: HashMap::new(),
            child_indices: HashMap::new(),
            hue_offset: 0.0,
            color_mode: ColorMode::Project, // Default to project coloring
            project_sessions: HashMap::new(),
            physics_enabled: true,
            hovered_node: None,
            selected_node: None,
            timeline: TimelineState::default(),
            temporal_attraction_enabled: true,
            temporal_window_secs: 300.0, // 5 minutes default
            max_temporal_edges: 100_000,
            max_tokens: 1,
            score_proximity_enabled: false,
            score_proximity_delta: 0.1,
            max_proximity_edges: 100_000,
            max_neighbors_per_node: 0,
        }
    }

    /// Normalize token count to 0-1 range using log scale
    /// Formula: log(tokens + 1) / log(max_tokens + 1)
    pub fn normalize_tokens(&self, node: &GraphNode) -> f32 {
        let tokens = node.total_tokens() as f32;
        let max = self.max_tokens as f32;
        if max <= 1.0 {
            return 0.5; // Default when no token data
        }
        (tokens + 1.0).ln() / (max + 1.0).ln()
    }

    /// Compute hue for a project based on its position in the directory tree.
    /// Tree distance maps to hue distance:
    /// - Siblings at each level get golden-ratio-spaced hues
    /// - Children inherit parent's base hue + smaller offset
    /// - Deeper nesting = tighter clustering (diminishing hue range)
    fn compute_project_hue(&mut self, project: &str) -> f32 {
        // Normalize consistently - just strip ~/
        let path = project.trim_start_matches("~/");
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        if parts.is_empty() {
            return 0.0;
        }

        let mut hue = 0.0;
        let mut parent_path = String::new();

        for (depth, part) in parts.iter().enumerate() {
            // Build current path
            let current_path = if parent_path.is_empty() {
                part.to_string()
            } else {
                format!("{}/{}", parent_path, part)
            };

            // Get or assign sibling index for this child under its parent
            let sibling_key = format!("{}:{}", parent_path, part);
            let sibling_idx = if let Some(&idx) = self.child_indices.get(&sibling_key) {
                idx
            } else {
                let parent_child_count = self.child_counts.entry(parent_path.clone()).or_insert(0);
                let idx = *parent_child_count;
                *parent_child_count += 1;
                self.child_indices.insert(sibling_key, idx);
                idx
            };

            // Golden ratio offset, scaled by depth
            // Depth 0: 360° range, depth 1: 180° range, depth 2: 90° range, etc.
            let range = 360.0 / (1 << depth) as f32;
            let offset = (sibling_idx as f32 * 137.5) % range;

            hue += offset;
            parent_path = current_path;
        }

        hue % 360.0
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
        self.child_counts.clear();
        self.child_indices.clear();
        self.project_sessions.clear();

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

            // Assign project color using tree-based hue assignment
            // Projects under the same parent directory get similar hues
            if !node.project.is_empty() && !self.project_colors.contains_key(&node.project) {
                let hue = self.compute_project_hue(&node.project);
                self.project_colors.insert(node.project.clone(), hue);
            }
        }

        // Build project_sessions mapping for hybrid coloring
        // Track earliest timestamp per session within each project
        let mut session_timestamps: HashMap<String, (String, f64)> = HashMap::new(); // session_id -> (project, min_ts)
        for node in data.nodes.iter() {
            if node.project.is_empty() {
                continue;
            }
            if let Some(ts) = node.timestamp_secs() {
                session_timestamps
                    .entry(node.session_id.clone())
                    .and_modify(|(_, existing_ts)| {
                        if ts < *existing_ts {
                            *existing_ts = ts;
                        }
                    })
                    .or_insert((node.project.clone(), ts));
            }
        }
        // Group sessions by project
        for (session_id, (project, ts)) in session_timestamps {
            self.project_sessions
                .entry(project)
                .or_default()
                .push((session_id, ts));
        }
        // Sort sessions within each project by timestamp
        for sessions in self.project_sessions.values_mut() {
            sessions.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        }

        // Compute max tokens for normalization
        self.max_tokens = data.nodes.iter()
            .map(|n| n.total_tokens())
            .max()
            .unwrap_or(1)
            .max(1); // Ensure non-zero for division

        self.data = data;
        self.physics_enabled = true;

        // Build timeline data
        self.build_timeline();

        // Build temporal edges (pre-computed at load time)
        if self.temporal_attraction_enabled {
            self.build_temporal_edges();
        }
    }

    /// Build pre-computed temporal edges between nodes close in time.
    /// Wrapper that builds edges for all nodes (no filtering).
    pub fn build_temporal_edges(&mut self) {
        self.build_temporal_edges_filtered(None);
    }

    /// Build pre-computed temporal edges, optionally restricted to a visible set.
    /// Uses sliding window algorithm: O(n) instead of O(n²).
    /// Caps at max_temporal_edges to prevent memory issues.
    /// When `visible` is Some, only nodes in the set participate in edge creation.
    pub fn build_temporal_edges_filtered(&mut self, visible: Option<&HashSet<String>>) {
        // Remove any existing temporal edges first
        self.data.edges.retain(|e| !e.is_temporal);

        if self.timeline.sorted_indices.is_empty() {
            return;
        }

        let window = self.temporal_window_secs;
        let max_edges = self.max_temporal_edges;

        // Build filtered sorted list: (original_sorted_pos, node_index, timestamp)
        // Only include nodes that are in the visible set (if provided)
        let filtered: Vec<(usize, f64)> = self.timeline.sorted_indices.iter().enumerate()
            .filter_map(|(i, &node_idx)| {
                if let Some(vis) = visible {
                    if !vis.contains(&self.data.nodes[node_idx].id) {
                        return None;
                    }
                }
                Some((node_idx, self.timeline.timestamps[i]))
            })
            .collect();

        let node_count = filtered.len();
        let mut temporal_edges = Vec::new();

        // Sliding window over sorted timestamps
        for i in 0..node_count {
            let (node_i_idx, ts_i) = filtered[i];

            for j in (i + 1)..node_count {
                let (node_j_idx, ts_j) = filtered[j];
                let dt = ts_j - ts_i;

                // Since sorted, if we exceed window we're done with this node
                if dt > window {
                    break;
                }

                // Strength decays linearly from 1.0 to 0.0 over the window
                let strength = 1.0 - (dt / window) as f32;

                let source_id = self.data.nodes[node_i_idx].id.clone();
                let target_id = self.data.nodes[node_j_idx].id.clone();

                temporal_edges.push(GraphEdge::temporal(source_id, target_id, strength));

                // Hard cap to prevent runaway memory issues
                if temporal_edges.len() >= max_edges {
                    eprintln!(
                        "Hit temporal edge limit of {} (window: {}s)",
                        max_edges,
                        window
                    );
                    self.data.edges.extend(temporal_edges);
                    return;
                }
            }
        }

        eprintln!(
            "Built {} temporal edges (window: {}s, nodes: {}/{}, limit: {})",
            temporal_edges.len(),
            window,
            node_count,
            self.timeline.sorted_indices.len(),
            max_edges
        );

        self.data.edges.extend(temporal_edges);
    }

    /// Rebuild temporal edges with a new window size
    pub fn set_temporal_window(&mut self, window_secs: f64, visible: Option<&HashSet<String>>) {
        self.temporal_window_secs = window_secs;
        if self.temporal_attraction_enabled {
            self.build_temporal_edges_filtered(visible);
        }
    }

    /// Toggle temporal attraction on/off
    pub fn set_temporal_attraction_enabled(&mut self, enabled: bool, visible: Option<&HashSet<String>>) {
        self.temporal_attraction_enabled = enabled;
        if enabled {
            self.build_temporal_edges_filtered(visible);
        } else {
            // Remove temporal edges
            self.data.edges.retain(|e| !e.is_temporal);
        }
    }

    /// Set maximum temporal edges and rebuild
    pub fn set_max_temporal_edges(&mut self, max_edges: usize, visible: Option<&HashSet<String>>) {
        self.max_temporal_edges = max_edges;
        if self.temporal_attraction_enabled {
            self.build_temporal_edges_filtered(visible);
        }
    }

    /// Replace all proximity (similarity) edges with the provided set.
    /// Removes existing similarity edges via retain, then extends with new ones.
    pub fn set_proximity_edges(&mut self, edges: Vec<GraphEdge>) {
        self.data.edges.retain(|e| !e.is_similarity);
        self.data.edges.extend(edges);
    }

    /// Build timeline sorted indices and timestamps for all item types.
    /// This creates a unified timeline that spans nodes, beads, and mail.
    fn build_timeline(&mut self) {
        // --- Build node timeline ---
        let mut timed_nodes: Vec<(usize, f64)> = self
            .data
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(i, node)| node.timestamp_secs().map(|t| (i, t)))
            .collect();
        timed_nodes.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        self.timeline.sorted_indices = timed_nodes.iter().map(|(i, _)| *i).collect();
        self.timeline.timestamps = timed_nodes.iter().map(|(_, t)| *t).collect();

        // --- Build bead timeline ---
        let mut timed_beads: Vec<(usize, f64)> = self
            .data
            .beads
            .iter()
            .enumerate()
            .filter_map(|(i, bead)| bead.timestamp_secs().map(|t| (i, t)))
            .collect();
        timed_beads.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        self.timeline.sorted_bead_indices = timed_beads.iter().map(|(i, _)| *i).collect();
        self.timeline.bead_timestamps = timed_beads.iter().map(|(_, t)| *t).collect();

        // --- Build mail timeline ---
        let mut timed_mail: Vec<(usize, f64)> = self
            .data
            .mail
            .iter()
            .enumerate()
            .filter_map(|(i, mail)| mail.timestamp_secs().map(|t| (i, t)))
            .collect();
        timed_mail.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        self.timeline.sorted_mail_indices = timed_mail.iter().map(|(i, _)| *i).collect();
        self.timeline.mail_timestamps = timed_mail.iter().map(|(_, t)| *t).collect();

        // --- Compute unified time range across all item types ---
        let mut min_time = f64::MAX;
        let mut max_time = f64::MIN;

        // Node timestamps
        if let Some(&first) = self.timeline.timestamps.first() {
            min_time = min_time.min(first);
        }
        if let Some(&last) = self.timeline.timestamps.last() {
            max_time = max_time.max(last);
        }

        // Bead timestamps
        if let Some(&first) = self.timeline.bead_timestamps.first() {
            min_time = min_time.min(first);
        }
        if let Some(&last) = self.timeline.bead_timestamps.last() {
            max_time = max_time.max(last);
        }

        // Mail timestamps
        if let Some(&first) = self.timeline.mail_timestamps.first() {
            min_time = min_time.min(first);
        }
        if let Some(&last) = self.timeline.mail_timestamps.last() {
            max_time = max_time.max(last);
        }

        // Set time range (only if we have valid data)
        if min_time < f64::MAX && max_time > f64::MIN {
            self.timeline.min_time = min_time;
            self.timeline.max_time = max_time;
        }

        // Initialize with all items visible
        self.timeline.position = 1.0;
        self.timeline.start_position = 0.0;
        self.update_visible_items();
    }

    /// Update which items are visible based on timeline position.
    /// This is the unified method that updates nodes, beads, and mail visibility.
    pub fn update_visible_items(&mut self) {
        let start_time = self.timeline.time_at_position(self.timeline.start_position);
        let end_time = self.timeline.time_at_position(self.timeline.position);

        // --- Update visible nodes ---
        self.timeline.visible_nodes.clear();
        for (i, &idx) in self.timeline.sorted_indices.iter().enumerate() {
            let t = self.timeline.timestamps[i];
            if t >= start_time && t <= end_time {
                if let Some(node) = self.data.nodes.get(idx) {
                    self.timeline.visible_nodes.insert(node.id.clone());
                }
            }
        }

        // --- Update visible beads ---
        self.timeline.visible_beads.clear();
        for (i, &idx) in self.timeline.sorted_bead_indices.iter().enumerate() {
            let t = self.timeline.bead_timestamps[i];
            if t >= start_time && t <= end_time {
                if let Some(bead) = self.data.beads.get(idx) {
                    self.timeline.visible_beads.insert(bead.id.clone());
                }
            }
        }

        // --- Update visible mail ---
        self.timeline.visible_mail.clear();
        for (i, &idx) in self.timeline.sorted_mail_indices.iter().enumerate() {
            let t = self.timeline.mail_timestamps[i];
            if t >= start_time && t <= end_time {
                if let Some(mail) = self.data.mail.get(idx) {
                    self.timeline.visible_mail.insert(mail.id.clone());
                }
            }
        }
    }

    /// Update which nodes are visible based on timeline position.
    /// This is a convenience wrapper that calls the unified update method.
    pub fn update_visible_nodes(&mut self) {
        self.update_visible_items();
    }

    /// Check if a node is visible in the current timeline window
    pub fn is_node_visible(&self, id: &str) -> bool {
        self.timeline.visible_nodes.contains(id)
    }

    /// Check if a bead is visible in the current timeline window
    pub fn is_bead_visible(&self, id: &str) -> bool {
        self.timeline.visible_beads.contains(id)
    }

    /// Check if a mail item is visible in the current timeline window
    pub fn is_mail_visible(&self, id: &str) -> bool {
        self.timeline.visible_mail.contains(id)
    }

    /// Check if an edge should be visible (both endpoints visible)
    pub fn is_edge_visible(&self, edge: &GraphEdge) -> bool {
        self.timeline.visible_nodes.contains(&edge.source)
            && self.timeline.visible_nodes.contains(&edge.target)
    }

    /// Get the current timeline window as (start_time, end_time) in epoch seconds.
    /// Useful for panels to filter their data.
    pub fn get_timeline_window(&self) -> (f64, f64) {
        let start_time = self.timeline.time_at_position(self.timeline.start_position);
        let end_time = self.timeline.time_at_position(self.timeline.position);
        (start_time, end_time)
    }

    /// Get counts of visible items for UI display
    pub fn visible_counts(&self) -> (usize, usize, usize, usize, usize, usize) {
        (
            self.timeline.visible_nodes.len(),
            self.data.nodes.len(),
            self.timeline.visible_beads.len(),
            self.data.beads.len(),
            self.timeline.visible_mail.len(),
            self.data.mail.len(),
        )
    }

    /// Get the position of a node
    pub fn get_pos(&self, id: &str) -> Option<Pos2> {
        self.positions.get(id).copied()
    }

    /// Get a node by ID
    pub fn get_node(&self, id: &str) -> Option<&GraphNode> {
        self.node_index.get(id).map(|&i| &self.data.nodes[i])
    }

    /// Apply global hue offset, wrapping around 360°
    pub fn apply_hue_offset(&self, hue: f32) -> f32 {
        (hue + self.hue_offset) % 360.0
    }

    /// Randomize the global hue offset (preserves relative color relationships)
    pub fn randomize_hue_offset(&mut self) {
        use rand::Rng;
        self.hue_offset = rand::thread_rng().gen_range(0.0..360.0);
    }

    /// Get the position (0.0-1.0) of a session within its project's timeline.
    /// Used for hybrid coloring: earlier sessions = 0.0, later sessions = 1.0.
    pub fn session_position_in_project(&self, session_id: &str, project: &str) -> f32 {
        if let Some(sessions) = self.project_sessions.get(project) {
            if sessions.len() <= 1 {
                return 0.5; // Single session, use middle
            }
            if let Some(idx) = sessions.iter().position(|(sid, _)| sid == session_id) {
                return idx as f32 / (sessions.len() - 1) as f32;
            }
        }
        0.5 // Default to middle if not found
    }

    /// Get the color for a node based on current color mode
    pub fn node_color(&self, node: &GraphNode) -> egui::Color32 {
        match self.color_mode {
            ColorMode::Project if !node.project.is_empty() => {
                let hue = self.project_colors.get(&node.project).copied().unwrap_or(0.0);
                hsl_to_rgb(self.apply_hue_offset(hue), 0.7, 0.55)
            }
            ColorMode::Hybrid if !node.project.is_empty() => {
                // Project hue + session position determines S/L
                let hue = self.project_colors.get(&node.project).copied().unwrap_or(0.0);
                let t = self.session_position_in_project(&node.session_id, &node.project);
                // Older sessions: lighter, less saturated (faded)
                // Newer sessions: darker, more saturated (prominent)
                let sat = 0.5 + t * 0.4;    // 0.5 -> 0.9
                let light = 0.65 - t * 0.2; // 0.65 -> 0.45
                hsl_to_rgb(self.apply_hue_offset(hue), sat, light)
            }
            _ => {
                // Session mode (or fallback for empty project)
                let hue = self.session_colors.get(&node.session_id).copied().unwrap_or(0.0);
                hsl_to_rgb(self.apply_hue_offset(hue), 0.7, 0.5)
            }
        }
    }

    /// Get a lighter version of node color (for fills)
    pub fn node_color_light(&self, node: &GraphNode) -> egui::Color32 {
        match self.color_mode {
            ColorMode::Project if !node.project.is_empty() => {
                let hue = self.project_colors.get(&node.project).copied().unwrap_or(0.0);
                hsl_to_rgb(self.apply_hue_offset(hue), 0.6, 0.75)
            }
            ColorMode::Hybrid if !node.project.is_empty() => {
                let hue = self.project_colors.get(&node.project).copied().unwrap_or(0.0);
                let t = self.session_position_in_project(&node.session_id, &node.project);
                // Lighter variant: shift both S and L up slightly
                let sat = 0.4 + t * 0.3;    // 0.4 -> 0.7
                let light = 0.8 - t * 0.15; // 0.8 -> 0.65
                hsl_to_rgb(self.apply_hue_offset(hue), sat, light)
            }
            _ => {
                let hue = self.session_colors.get(&node.session_id).copied().unwrap_or(0.0);
                hsl_to_rgb(self.apply_hue_offset(hue), 0.6, 0.7)
            }
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
        } else {
            match self.color_mode {
                ColorMode::Project => {
                    // Find source node's project for edge color
                    if let Some(node) = self.get_node(&edge.source) {
                        if !node.project.is_empty() {
                            let hue = self.project_colors.get(&node.project).copied().unwrap_or(0.0);
                            return hsl_to_rgb(self.apply_hue_offset(hue), 0.5, 0.4);
                        }
                    }
                    // Fallback to session color
                    let hue = self.session_colors.get(&edge.session_id).copied().unwrap_or(0.0);
                    hsl_to_rgb(self.apply_hue_offset(hue), 0.5, 0.4)
                }
                ColorMode::Hybrid => {
                    // Use source node's hybrid coloring
                    if let Some(node) = self.get_node(&edge.source) {
                        if !node.project.is_empty() {
                            let hue = self.project_colors.get(&node.project).copied().unwrap_or(0.0);
                            let t = self.session_position_in_project(&node.session_id, &node.project);
                            let sat = 0.4 + t * 0.3;
                            let light = 0.5 - t * 0.15;
                            return hsl_to_rgb(self.apply_hue_offset(hue), sat, light);
                        }
                    }
                    let hue = self.session_colors.get(&edge.session_id).copied().unwrap_or(0.0);
                    hsl_to_rgb(self.apply_hue_offset(hue), 0.5, 0.4)
                }
                ColorMode::Session => {
                    let hue = self.session_colors.get(&edge.session_id).copied().unwrap_or(0.0);
                    hsl_to_rgb(self.apply_hue_offset(hue), 0.7, 0.5)
                }
            }
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

/// Convert a color to greyscale (using luminosity method)
pub fn to_greyscale(color: egui::Color32) -> egui::Color32 {
    let r = color.r() as f32;
    let g = color.g() as f32;
    let b = color.b() as f32;
    // Luminosity formula for perceived brightness
    let grey = (0.299 * r + 0.587 * g + 0.114 * b) as u8;
    egui::Color32::from_rgba_unmultiplied(grey, grey, grey, color.a())
}

/// Desaturate a color by blending it towards grey
/// amount: 0.0 = original color, 1.0 = fully grey
pub fn desaturate(color: egui::Color32, amount: f32) -> egui::Color32 {
    let grey = to_greyscale(color);
    let inv = 1.0 - amount;
    let r = (color.r() as f32 * inv + grey.r() as f32 * amount) as u8;
    let g = (color.g() as f32 * inv + grey.g() as f32 * amount) as u8;
    let b = (color.b() as f32 * inv + grey.b() as f32 * amount) as u8;
    egui::Color32::from_rgba_unmultiplied(r, g, b, color.a())
}

/// Linearly interpolate between two colors
/// t: 0.0 = color a, 1.0 = color b
pub fn lerp_color(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    egui::Color32::from_rgba_unmultiplied(
        (a.r() as f32 * (1.0 - t) + b.r() as f32 * t) as u8,
        (a.g() as f32 * (1.0 - t) + b.g() as f32 * t) as u8,
        (a.b() as f32 * (1.0 - t) + b.b() as f32 * t) as u8,
        (a.a() as f32 * (1.0 - t) + b.a() as f32 * t) as u8,
    )
}

// ============================================================================
// Histogram Data Structures
// ============================================================================

/// How to display token counts in the histogram
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TokenDisplayMode {
    /// Show raw token counts
    #[default]
    Absolute,
    /// Show as percentage of total tokens in the time window
    Percentage,
    /// Show as rate (tokens per minute)
    Rate,
}

impl TokenDisplayMode {
    pub fn label(&self) -> &'static str {
        match self {
            TokenDisplayMode::Absolute => "Absolute",
            TokenDisplayMode::Percentage => "Percentage",
            TokenDisplayMode::Rate => "Rate",
        }
    }
}

/// How to order/stack bars in the histogram
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StackOrder {
    /// Stack by token type: input, output, cache_read, cache_creation
    #[default]
    ByTokenType,
    /// Stack by role: user, assistant
    ByRole,
    /// Stack by project
    ByProject,
    /// Stack by session
    BySession,
}

impl StackOrder {
    pub fn label(&self) -> &'static str {
        match self {
            StackOrder::ByTokenType => "Token Type",
            StackOrder::ByRole => "Role",
            StackOrder::ByProject => "Project",
            StackOrder::BySession => "Session",
        }
    }
}

/// Filter criteria for histogram data
#[derive(Debug, Clone, Default)]
pub struct HistogramFilter {
    /// Only include specific projects (empty = all)
    pub projects: Vec<String>,
    /// Only include specific sessions (empty = all)
    pub sessions: Vec<String>,
    /// Only include specific roles (empty = all)
    pub roles: Vec<Role>,
    /// Include input tokens
    pub include_input: bool,
    /// Include output tokens
    pub include_output: bool,
    /// Include cache read tokens
    pub include_cache_read: bool,
    /// Include cache creation tokens
    pub include_cache_creation: bool,
}

impl HistogramFilter {
    /// Create a filter that includes all token types
    pub fn all() -> Self {
        Self {
            projects: Vec::new(),
            sessions: Vec::new(),
            roles: Vec::new(),
            include_input: true,
            include_output: true,
            include_cache_read: true,
            include_cache_creation: true,
        }
    }
}

/// A segment within a histogram bin (for stacked bars)
#[derive(Debug, Clone)]
pub struct TokenSegment {
    /// Label for this segment (e.g., "input", "output", project name, etc.)
    pub label: String,
    /// Token count for this segment
    pub count: i64,
    /// Color for this segment
    pub color: egui::Color32,
}

/// A single bin in the histogram
#[derive(Debug, Clone)]
pub struct TokenBin {
    /// Start time of this bin (epoch seconds)
    pub start_time: f64,
    /// End time of this bin (epoch seconds)
    pub end_time: f64,
    /// Segments within this bin (for stacked bars)
    pub segments: Vec<TokenSegment>,
    /// Total token count across all segments
    pub total: i64,
}

impl TokenBin {
    /// Create a new empty bin
    pub fn new(start_time: f64, end_time: f64) -> Self {
        Self {
            start_time,
            end_time,
            segments: Vec::new(),
            total: 0,
        }
    }

    /// Add a segment to this bin
    pub fn add_segment(&mut self, label: String, count: i64, color: egui::Color32) {
        self.segments.push(TokenSegment { label, count, color });
        self.total += count;
    }

    /// Get the midpoint time of this bin
    pub fn midpoint(&self) -> f64 {
        (self.start_time + self.end_time) / 2.0
    }

    /// Get the duration of this bin in seconds
    pub fn duration(&self) -> f64 {
        self.end_time - self.start_time
    }
}

/// State for the token usage histogram
#[derive(Debug, Clone)]
pub struct HistogramState {
    /// Computed histogram bins
    pub bins: Vec<TokenBin>,
    /// Number of bins to divide the time range into
    pub bin_count: usize,
    /// How to display values
    pub display_mode: TokenDisplayMode,
    /// How to stack/order bars
    pub stack_order: StackOrder,
    /// Filter criteria
    pub filter: HistogramFilter,
    /// Maximum value across all bins (for normalization)
    pub max_value: i64,
    /// Total tokens in the current view
    pub total_tokens: i64,
    /// Whether the histogram needs rebuilding
    pub dirty: bool,
}

impl Default for HistogramState {
    fn default() -> Self {
        Self {
            bins: Vec::new(),
            bin_count: 20,
            display_mode: TokenDisplayMode::default(),
            stack_order: StackOrder::default(),
            filter: HistogramFilter::all(),
            max_value: 0,
            total_tokens: 0,
            dirty: true,
        }
    }
}

impl HistogramState {
    /// Mark the histogram as needing a rebuild
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Check if rebuild is needed
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Clear the dirty flag after rebuilding
    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    /// Set the number of bins and mark dirty
    pub fn set_bin_count(&mut self, count: usize) {
        if self.bin_count != count {
            self.bin_count = count.max(1);
            self.dirty = true;
        }
    }

    /// Set display mode and mark dirty
    pub fn set_display_mode(&mut self, mode: TokenDisplayMode) {
        if self.display_mode != mode {
            self.display_mode = mode;
            self.dirty = true;
        }
    }

    /// Set stack order and mark dirty
    pub fn set_stack_order(&mut self, order: StackOrder) {
        if self.stack_order != order {
            self.stack_order = order;
            self.dirty = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_iso_timestamp() {
        // Full datetime with timezone
        let ts = parse_iso_timestamp("2025-12-31T01:30:07.726213+00:00");
        assert!(ts.is_some());
        let secs = ts.unwrap();
        // Should be around 2025-12-31 01:30:07 UTC in epoch seconds
        // 2025 is roughly 55 years after 1970, so ~55 * 365.25 * 86400 ≈ 1.735 billion
        assert!(secs > 1_700_000_000.0, "timestamp should be in 2025 range");
        assert!(secs < 1_800_000_000.0, "timestamp should be in 2025 range");

        // Date-only format
        let ts_date = parse_iso_timestamp("2025-12-31");
        assert!(ts_date.is_some());
        let secs_date = ts_date.unwrap();
        // Should be midnight of 2025-12-31
        assert!(secs_date > 1_700_000_000.0);
        assert!(secs_date < 1_800_000_000.0);

        // Invalid input
        let ts_invalid = parse_iso_timestamp("not-a-date");
        assert!(ts_invalid.is_none());
    }

    #[test]
    fn test_timeline_state_default() {
        let ts = TimelineState::default();
        assert!(ts.visible_nodes.is_empty());
        assert!(ts.visible_beads.is_empty());
        assert!(ts.visible_mail.is_empty());
        assert_eq!(ts.position, 1.0);
        assert_eq!(ts.start_position, 0.0);
    }

    #[test]
    fn test_timeline_time_position_conversion() {
        let mut ts = TimelineState::default();
        ts.min_time = 1000.0;
        ts.max_time = 2000.0;

        // Position 0.0 should be min_time
        assert_eq!(ts.time_at_position(0.0), 1000.0);
        // Position 1.0 should be max_time
        assert_eq!(ts.time_at_position(1.0), 2000.0);
        // Position 0.5 should be midpoint
        assert_eq!(ts.time_at_position(0.5), 1500.0);

        // Reverse: time to position
        assert_eq!(ts.position_at_time(1000.0), 0.0);
        assert_eq!(ts.position_at_time(2000.0), 1.0);
        assert_eq!(ts.position_at_time(1500.0), 0.5);
    }

    #[test]
    fn test_bead_item_timestamp() {
        let bead = BeadItem {
            id: "test-1".to_string(),
            title: "Test Bead".to_string(),
            status: IssueStatus::Open,
            labels: vec![],
            priority: 2,
            created_at: Some("2025-06-15T12:00:00+00:00".to_string()),
            updated_at: Some("2025-06-16T12:00:00+00:00".to_string()),
            issue_type: None,
            description: None,
            assignee: None,
        };

        // Should have valid timestamps
        assert!(bead.timestamp_secs().is_some());
        assert!(bead.updated_at_secs().is_some());

        // Updated should be after created
        assert!(bead.updated_at_secs().unwrap() > bead.timestamp_secs().unwrap());
    }

    #[test]
    fn test_mail_item_timestamp() {
        let mail = MailItem {
            id: "mail-1".to_string(),
            subject: "Test Mail".to_string(),
            sender: "sender@test.com".to_string(),
            recipient: "recipient@test.com".to_string(),
            timestamp: Some("2025-06-15T12:00:00+00:00".to_string()),
            thread_id: None,
            is_unread: true,
            preview: None,
        };

        // Should have valid timestamp
        assert!(mail.timestamp_secs().is_some());
    }

    /// Helper: create a GraphNode with a given id and timestamp
    fn make_node(id: &str, timestamp: &str) -> GraphNode {
        GraphNode {
            id: id.to_string(),
            role: Role::User,
            content_preview: String::new(),
            full_content: None,
            session_id: "s1".to_string(),
            session_short: "s1".to_string(),
            project: "proj".to_string(),
            timestamp: Some(timestamp.to_string()),
            importance_score: None,
            importance_reason: None,
            output_tokens: None,
            input_tokens: None,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            semantic_filter_matches: vec![],
            has_tool_usage: false,
        }
    }

    /// Helper: create a GraphState loaded with nodes and timeline built
    fn make_graph_with_nodes(nodes: Vec<GraphNode>) -> GraphState {
        let mut graph = GraphState::new();
        let data = GraphData {
            nodes,
            edges: vec![],
            beads: vec![],
            mail: vec![],
        };
        let bounds = egui::Rect::from_center_size(
            egui::Pos2::new(400.0, 300.0),
            egui::Vec2::new(600.0, 400.0),
        );
        // Disable temporal edges during load so we control them manually
        graph.temporal_attraction_enabled = false;
        graph.load(data, bounds);
        graph
    }

    #[test]
    fn test_build_temporal_edges_unfiltered() {
        let nodes = vec![
            make_node("A", "2025-06-15T12:00:00+00:00"),
            make_node("B", "2025-06-15T12:01:00+00:00"), // 60s after A
            make_node("C", "2025-06-15T12:10:00+00:00"), // 10min after A
        ];
        let mut graph = make_graph_with_nodes(nodes);
        graph.temporal_window_secs = 120.0; // 2 minute window
        graph.max_temporal_edges = 1000;

        graph.build_temporal_edges_filtered(None);

        let temporal: Vec<_> = graph.data.edges.iter().filter(|e| e.is_temporal).collect();
        // A-B within 2min window, A-C and B-C outside 2min window
        assert_eq!(temporal.len(), 1);
        assert_eq!(temporal[0].source, "A");
        assert_eq!(temporal[0].target, "B");
    }

    #[test]
    fn test_build_temporal_edges_filtered_excludes_invisible() {
        let nodes = vec![
            make_node("A", "2025-06-15T12:00:00+00:00"),
            make_node("B", "2025-06-15T12:01:00+00:00"),
            make_node("C", "2025-06-15T12:01:30+00:00"),
        ];
        let mut graph = make_graph_with_nodes(nodes);
        graph.temporal_window_secs = 120.0;
        graph.max_temporal_edges = 1000;

        // Only A and C are visible (B is filtered out)
        let visible: HashSet<String> = ["A", "C"].iter().map(|s| s.to_string()).collect();
        graph.build_temporal_edges_filtered(Some(&visible));

        let temporal: Vec<_> = graph.data.edges.iter().filter(|e| e.is_temporal).collect();
        // A-C is 90s apart, within 120s window, so 1 edge
        assert_eq!(temporal.len(), 1);
        assert_eq!(temporal[0].source, "A");
        assert_eq!(temporal[0].target, "C");
    }

    #[test]
    fn test_build_temporal_edges_filtered_empty_set() {
        let nodes = vec![
            make_node("A", "2025-06-15T12:00:00+00:00"),
            make_node("B", "2025-06-15T12:01:00+00:00"),
        ];
        let mut graph = make_graph_with_nodes(nodes);
        graph.temporal_window_secs = 600.0;
        graph.max_temporal_edges = 1000;

        // Empty visible set — no edges should be created
        let visible: HashSet<String> = HashSet::new();
        graph.build_temporal_edges_filtered(Some(&visible));

        let temporal_count = graph.data.edges.iter().filter(|e| e.is_temporal).count();
        assert_eq!(temporal_count, 0);
    }

    #[test]
    fn test_build_temporal_edges_filtered_respects_max_edges() {
        // Create many nodes close together to generate lots of edges
        let nodes: Vec<GraphNode> = (0..20)
            .map(|i| make_node(
                &format!("N{}", i),
                &format!("2025-06-15T12:00:{:02}+00:00", i),
            ))
            .collect();
        let mut graph = make_graph_with_nodes(nodes);
        graph.temporal_window_secs = 60.0; // all within window
        graph.max_temporal_edges = 5; // strict cap

        graph.build_temporal_edges_filtered(None);

        let temporal_count = graph.data.edges.iter().filter(|e| e.is_temporal).count();
        assert_eq!(temporal_count, 5);
    }

    #[test]
    fn test_build_temporal_edges_filtered_cleans_old_edges() {
        let nodes = vec![
            make_node("A", "2025-06-15T12:00:00+00:00"),
            make_node("B", "2025-06-15T12:01:00+00:00"),
        ];
        let mut graph = make_graph_with_nodes(nodes);
        graph.temporal_window_secs = 120.0;
        graph.max_temporal_edges = 1000;

        // Build once
        graph.build_temporal_edges_filtered(None);
        assert_eq!(graph.data.edges.iter().filter(|e| e.is_temporal).count(), 1);

        // Build again — should still be 1 (old temporal edges removed first)
        graph.build_temporal_edges_filtered(None);
        assert_eq!(graph.data.edges.iter().filter(|e| e.is_temporal).count(), 1);
    }

    #[test]
    fn test_set_temporal_window_with_visible_set() {
        let nodes = vec![
            make_node("A", "2025-06-15T12:00:00+00:00"),
            make_node("B", "2025-06-15T12:01:00+00:00"),
            make_node("C", "2025-06-15T12:05:00+00:00"),
        ];
        let mut graph = make_graph_with_nodes(nodes);
        graph.temporal_attraction_enabled = true;
        graph.max_temporal_edges = 1000;

        // Set window to 2 min, only A and C visible
        let visible: HashSet<String> = ["A", "C"].iter().map(|s| s.to_string()).collect();
        graph.set_temporal_window(120.0, Some(&visible));

        let temporal: Vec<_> = graph.data.edges.iter().filter(|e| e.is_temporal).collect();
        // A-C is 5 min apart, outside 2 min window → 0 edges
        assert_eq!(temporal.len(), 0);

        // Widen window to 10 min
        graph.set_temporal_window(600.0, Some(&visible));
        let temporal: Vec<_> = graph.data.edges.iter().filter(|e| e.is_temporal).collect();
        // A-C is 5 min apart, within 10 min window → 1 edge
        assert_eq!(temporal.len(), 1);
    }
}
