//! Main application state and UI.

use crate::api::{ApiClient, EmbeddingGenResult, EmbeddingStats, FilterStatusResponse, IngestResult, RescoreEvent, RescoreProgress, RescoreResult};
use crate::db::DbClient;
use crate::graph::types::{ColorMode, FilterMode, GraphEdge, NeighborhoodSummaryData, PartialSummaryData, SemanticFilter, SemanticFilterMode, SessionSummaryData};
use crate::graph::{ForceLayout, GraphState};
use crate::mail::{MailNetworkState, render_mail_network};
use crate::project_tree::{self, CheckState, ProjectTreeNode};
use crate::settings::{Preset, Settings, SidebarTab, SizingPreset};
use crate::theme;
use eframe::egui::{self, Color32, Pos2, Stroke, Vec2};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver};
use std::time::{Instant, SystemTime};

/// Fixed 8-color palette for proximity query edges
const QUERY_COLORS: [Color32; 8] = [
    Color32::from_rgb(6, 182, 212),    // Cyan (original)
    Color32::from_rgb(249, 115, 22),   // Orange
    Color32::from_rgb(168, 85, 247),   // Purple
    Color32::from_rgb(34, 197, 94),    // Green
    Color32::from_rgb(239, 68, 68),    // Red
    Color32::from_rgb(234, 179, 8),    // Yellow
    Color32::from_rgb(236, 72, 153),   // Pink
    Color32::from_rgb(59, 130, 246),   // Blue
];

/// A single proximity (semantic edge) query with its own color, scores, and edges
struct ProximityQuery {
    query: String,
    color: Color32,
    scores: HashMap<String, f32>,
    edges: Vec<GraphEdge>,
    edge_count: usize,
    active: bool,
    loading: bool,
    rx: Option<Receiver<Result<(Vec<GraphEdge>, HashMap<String, f32>), String>>>,
}

/// Which edge popup is currently open (gear icon popups)
#[derive(Debug, Clone, Copy, PartialEq)]
enum EdgePopup {
    Physics,
    LayoutShaping,
    TemporalClustering,
    ScoreProximity,
}

/// Time range options for filtering
/// Format hours into a human-readable time range label
fn format_hours_label(hours: f32) -> String {
    if hours < 2.0 {
        format!("{:.0} hour", hours)
    } else if hours < 48.0 {
        format!("{:.0} hours", hours)
    } else if hours < 336.0 {
        let days = hours / 24.0;
        if days == days.round() {
            format!("{:.0} days", days)
        } else {
            format!("{:.1} days", days)
        }
    } else if hours < 1440.0 {
        let weeks = hours / 168.0;
        if weeks == weeks.round() {
            format!("{:.0} weeks", weeks)
        } else {
            format!("{:.1} weeks", weeks)
        }
    } else {
        let months = hours / 720.0;
        if months == months.round() {
            format!("{:.0} months", months)
        } else {
            format!("{:.1} months", months)
        }
    }
}

/// Get histogram bin duration in seconds for a given time range in hours.
/// Snaps to the nearest "nice" interval, targeting ~20 bins.
fn bin_duration_for_hours(hours: f32) -> f64 {
    let total_secs = hours as f64 * 3600.0;
    let raw_bin = total_secs / 20.0;
    let nice: &[f64] = &[
        60.0,             // 1 min
        5.0 * 60.0,      // 5 min
        15.0 * 60.0,     // 15 min
        30.0 * 60.0,     // 30 min
        3600.0,           // 1 hr
        3.0 * 3600.0,    // 3 hr
        6.0 * 3600.0,    // 6 hr
        12.0 * 3600.0,   // 12 hr
        24.0 * 3600.0,   // 1 day
        7.0 * 24.0 * 3600.0, // 1 week
    ];
    nice.iter()
        .copied()
        .min_by(|a, b| (a - raw_bin).abs().partial_cmp(&(b - raw_bin).abs()).unwrap())
        .unwrap_or(raw_bin)
}

/// Importance scoring statistics
#[derive(Debug, Clone)]
pub struct ImportanceStats {
    pub total_messages: i64,
    pub scored_messages: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum HistogramStackOrder {
    #[default]
    MostTokens,
    OldestFirst,
    MostMessages,
}

impl HistogramStackOrder {
    fn label(&self) -> &'static str {
        match self {
            Self::MostTokens => "Most Tokens",
            Self::OldestFirst => "Oldest First",
            Self::MostMessages => "Most Messages",
        }
    }
}

#[derive(Debug, Clone)]
struct SessionTokens {
    session_id: String,
    project: String,
    total_tokens: i64,
    is_filtered: bool,
}

struct TokenBin {
    timestamp_start: String,
    timestamp_end: String,
    sessions: Vec<SessionTokens>,
    total_tokens: i64,
}

/// Main dashboard application
pub struct DashboardApp {
    // Database client
    db: Option<DbClient>,
    db_connected: bool,
    db_error: Option<String>,

    // Graph state
    graph: GraphState,
    layout: ForceLayout,

    // UI state
    sidebar_tab: SidebarTab,
    time_range_hours: f32,       // currently loaded time range
    slider_hours: f32,           // pending slider value (before confirm)
    node_size: f32,
    show_arrows: bool,
    loading: bool,
    timeline_enabled: bool,
    timeline_histogram_mode: bool,
    hover_scrubs_timeline: bool,

    // Node sizing (unified formula)
    sizing_preset: SizingPreset,
    w_importance: f32,
    w_tokens: f32,
    w_time: f32,
    max_node_multiplier: f32,

    // Temporal edge opacity
    temporal_edge_opacity: f32,

    // Importance filtering
    importance_threshold: f32,
    importance_filter: FilterMode,
    importance_stats: Option<ImportanceStats>,
    rescore_loading: bool,
    rescore_receiver: Option<Receiver<RescoreEvent>>,
    rescore_result: Option<RescoreResult>,
    rescore_progress: Option<RescoreProgress>,

    // Tool usage filtering
    tool_use_filter: FilterMode,
    bypass_edges: Vec<crate::graph::types::GraphEdge>,

    // Project filtering
    project_filter: FilterMode,
    selected_projects: HashSet<String>,
    available_projects: Vec<String>,
    project_tree: Option<ProjectTreeNode>,
    /// Tracks which tree nodes are expanded in the UI (by full_path).
    project_tree_expanded: HashSet<String>,

    // Debug tooltip
    debug_tooltip: bool,

    // Viewport state
    pan_offset: Vec2,
    zoom: f32,
    dragging: bool,
    drag_start: Option<Pos2>,

    // Timeline dragging state
    timeline_dragging: bool,
    last_playback_time: Instant,

    // Performance tracking
    last_frame: Instant,
    frame_times: Vec<f32>,
    fps: f32,

    // Summary panel state (point-in-time)
    summary_node_id: Option<String>,
    summary_session_id: Option<String>,
    summary_timestamp: Option<String>,
    summary_loading: bool,
    summary_data: Option<PartialSummaryData>,
    summary_error: Option<String>,
    summary_receiver: Option<Receiver<Result<PartialSummaryData, String>>>,

    // Session summary state (full session)
    session_summary_data: Option<SessionSummaryData>,
    session_summary_loading: bool,
    session_summary_receiver: Option<Receiver<Result<SessionSummaryData, String>>>,

    // Neighborhood summary state (Ctrl+Click cluster)
    neighborhood_summary_data: Option<NeighborhoodSummaryData>,
    neighborhood_summary_loading: bool,
    neighborhood_summary_error: Option<String>,
    neighborhood_summary_receiver: Option<Receiver<Result<NeighborhoodSummaryData, String>>>,
    neighborhood_summary_center_node: Option<String>,
    neighborhood_summary_count: usize,
    neighborhood_depth: usize,
    neighborhood_include_temporal: bool,

    // Cmd+Hover neighborhood preview
    cmd_hover_neighbors: HashSet<String>,

    // Floating summary window state
    summary_window_open: bool,
    summary_window_dragged: bool,

    // Floating neighborhood window state
    neighborhood_window_open: bool,
    neighborhood_window_dragged: bool,

    // Resume session clipboard feedback
    resume_copied_at: Option<Instant>,

    // Double-click detection
    last_click_time: Instant,
    last_click_node: Option<String>,

    // Settings persistence
    settings: Settings,
    settings_dirty: bool,
    last_settings_save: Instant,

    // Preset management
    preset_name_input: String,
    selected_preset_index: Option<usize>,

    // Semantic filters
    semantic_filters: Vec<SemanticFilter>,
    semantic_filter_modes: HashMap<i32, SemanticFilterMode>,
    new_filter_input: String,
    semantic_filter_loading: bool,
    categorizing_filter_id: Option<i32>,
    categorization_receiver: Option<Receiver<Result<(), String>>>,
    categorization_progress_rx: Option<Receiver<FilterStatusResponse>>,
    categorization_progress: Option<(i64, i64)>, // (scored, total)
    categorization_done_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,

    // Cached semantic filter visible set (invalidate on filter change or data load)
    semantic_filter_cache: Option<HashSet<String>>,

    // Expanded filter detail panels (toggled by clicking filter name)
    expanded_filter_ids: HashSet<i32>,

    // Score-proximity edges (unified similarity search + clustering)
    proximity_queries: Vec<ProximityQuery>,
    proximity_input: String,
    proximity_heat_map_index: Option<usize>,  // None = max across all queries
    proximity_edge_opacity: f32,
    proximity_edge_count_filtered: usize,
    proximity_stiffness: f32,
    embedding_stats: Option<EmbeddingStats>,
    embedding_gen_loading: bool,
    embedding_gen_receiver: Option<Receiver<Result<EmbeddingGenResult, String>>>,

    // Summary caches (populated on double-click, shown in tooltip)
    point_in_time_summary_cache: HashMap<String, PartialSummaryData>,  // node_id -> summary
    session_summary_cache: HashMap<String, SessionSummaryData>,        // session_id -> summary

    // Refresh & sync state
    last_synced: Option<Instant>,
    beads_last_check: Instant,
    beads_last_mtime: Option<SystemTime>,

    // Mail network graph (agent communication)
    mail_network_state: Option<MailNetworkState>,
    mail_network_loading: bool,
    mail_network_error: Option<String>,

    // Collapsible side panels
    beads_panel_open: bool,
    mail_panel_open: bool,

    // Token histogram panel
    histogram_panel_enabled: bool,
    histogram_split_ratio: f32,
    histogram_dragging_divider: bool,
    histogram_hovered_bin: Option<usize>,
    histogram_bar_width: f32,
    histogram_scroll_offset: f32,
    histogram_stack_order: HistogramStackOrder,
    histogram_last_clicked: Option<(String, String)>, // (session_id, project)
    histogram_drill_level: u8, // 0=none, 1=project, 2=session
    histogram_session_filter: Option<String>, // session_id to isolate
    session_metadata_cache: HashMap<String, (f64, usize)>,

    // Layout shaping (directed stiffness + recency centering)
    layout_shaping_enabled: bool,

    // Edge popup state (gear icon popups)
    edge_popup_open: Option<EdgePopup>,

    // Session ingest (re-import from ~/.claude/)
    ingest_loading: bool,
    ingest_receiver: Option<Receiver<Result<IngestResult, String>>>,
    ingest_result: Option<IngestResult>,

    // Effective visible set: unified set of nodes passing ALL filters
    // Rebuilt when any filter changes (dirty flag), used for edge rendering,
    // hover detection, physics, counters, and temporal edge rebuilds.
    effective_visible_nodes: HashSet<String>,
    effective_visible_count: usize,
    effective_visible_dirty: bool,
    temporal_edges_dirty: bool,
}

impl DashboardApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Configure fonts - add emoji support
        // egui's default font doesn't include emoji glyphs, so we load NotoEmoji
        // as a fallback font for both Proportional and Monospace families.
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "NotoEmoji".to_owned(),
            egui::FontData::from_static(include_bytes!(
                "../assets/fonts/NotoEmoji-Regular.ttf"
            )),
        );
        // Add NotoEmoji as fallback (after the default fonts) for both families
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .push("NotoEmoji".to_owned());
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .push("NotoEmoji".to_owned());
        cc.egui_ctx.set_fonts(fonts);

        // Load saved settings
        let settings = Settings::load();

        // Create layout with saved physics settings
        let mut layout = ForceLayout::default();
        layout.repulsion = settings.repulsion;
        layout.attraction = settings.attraction;
        layout.centering = settings.centering;
        layout.temporal_strength = settings.temporal_strength;
        layout.size_physics_weight = settings.size_physics_weight;
        layout.directed_stiffness = settings.directed_stiffness;
        layout.recency_centering = settings.recency_centering;
        layout.momentum = settings.momentum;

        // Create graph state with saved settings
        let mut graph = GraphState::new();
        graph.physics_enabled = settings.physics_enabled;
        graph.color_mode = settings.color_mode;
        graph.temporal_attraction_enabled = settings.temporal_attraction_enabled;
        graph.temporal_window_secs = settings.temporal_window_mins as f64 * 60.0;
        graph.max_temporal_edges = settings.max_temporal_edges;

        // Try to connect to database
        let (db, db_connected, db_error) = match DbClient::new() {
            Ok(client) => (Some(client), true, None),
            Err(e) => (None, false, Some(e)),
        };

        let mut app = Self {
            db,
            db_connected,
            db_error,
            graph,
            layout,
            sidebar_tab: settings.sidebar_tab,
            time_range_hours: settings.time_range_hours,
            slider_hours: settings.time_range_hours,
            node_size: settings.node_size,
            show_arrows: settings.show_arrows,
            loading: false,
            timeline_enabled: settings.timeline_enabled,
            timeline_histogram_mode: false, // Default to notch view
            hover_scrubs_timeline: settings.hover_scrubs_timeline,
            sizing_preset: settings.sizing_preset,
            w_importance: settings.w_importance,
            w_tokens: settings.w_tokens,
            w_time: settings.w_time,
            max_node_multiplier: settings.max_node_multiplier,
            temporal_edge_opacity: settings.temporal_edge_opacity,
            importance_threshold: settings.importance_threshold,
            importance_filter: settings.importance_filter,
            importance_stats: None,
            rescore_loading: false,
            rescore_receiver: None,
            rescore_result: None,
            rescore_progress: None,
            tool_use_filter: settings.tool_use_filter,
            bypass_edges: Vec::new(),
            project_filter: settings.project_filter,
            selected_projects: HashSet::new(),
            project_tree: None,
            project_tree_expanded: HashSet::new(),
            available_projects: Vec::new(),
            debug_tooltip: false,
            pan_offset: Vec2::ZERO,
            zoom: 1.0,
            dragging: false,
            drag_start: None,
            timeline_dragging: false,
            last_playback_time: Instant::now(),
            last_frame: Instant::now(),
            frame_times: Vec::with_capacity(60),
            fps: 0.0,

            // Summary panel state (point-in-time)
            summary_node_id: None,
            summary_session_id: None,
            summary_timestamp: None,
            summary_loading: false,
            summary_data: None,
            summary_error: None,
            summary_receiver: None,

            // Session summary state (full session)
            session_summary_data: None,
            session_summary_loading: false,
            session_summary_receiver: None,

            // Neighborhood summary state
            neighborhood_summary_data: None,
            neighborhood_summary_loading: false,
            neighborhood_summary_error: None,
            neighborhood_summary_receiver: None,
            neighborhood_summary_center_node: None,
            neighborhood_summary_count: 0,
            neighborhood_depth: 1,
            neighborhood_include_temporal: true,

            // Cmd+Hover neighborhood preview
            cmd_hover_neighbors: HashSet::new(),

            // Floating summary window state
            summary_window_open: false,
            summary_window_dragged: false,

            // Floating neighborhood window state
            neighborhood_window_open: false,
            neighborhood_window_dragged: false,

            // Resume session clipboard feedback
            resume_copied_at: None,

            // Double-click detection
            last_click_time: Instant::now(),
            last_click_node: None,

            // Collapsible side panels (read before settings move)
            beads_panel_open: settings.beads_panel_open,
            mail_panel_open: settings.mail_panel_open,

            // Token histogram panel
            histogram_panel_enabled: settings.histogram_panel_enabled,
            histogram_split_ratio: settings.histogram_split_ratio,
            histogram_dragging_divider: false,
            histogram_hovered_bin: None,
            histogram_bar_width: 40.0,
            histogram_scroll_offset: 0.0,
            histogram_stack_order: HistogramStackOrder::MostTokens,
            histogram_last_clicked: None,
            histogram_drill_level: 0,
            histogram_session_filter: None,
            session_metadata_cache: HashMap::new(),

            // Settings persistence
            settings,
            settings_dirty: false,
            last_settings_save: Instant::now(),

            // Preset management
            preset_name_input: String::new(),
            selected_preset_index: None,

            // Semantic filters
            semantic_filters: Vec::new(),
            semantic_filter_modes: HashMap::new(),
            new_filter_input: String::new(),
            semantic_filter_loading: false,
            categorizing_filter_id: None,
            categorization_receiver: None,
            categorization_progress_rx: None,
            categorization_progress: None,
            categorization_done_flag: None,
            semantic_filter_cache: None,
            expanded_filter_ids: HashSet::new(),

            // Score-proximity edges
            proximity_queries: Vec::new(),
            proximity_input: String::new(),
            proximity_heat_map_index: None,
            proximity_edge_opacity: 0.3,
            proximity_edge_count_filtered: 0,
            proximity_stiffness: 1.0,
            embedding_stats: None,
            embedding_gen_loading: false,
            embedding_gen_receiver: None,

            // Summary caches
            point_in_time_summary_cache: HashMap::new(),
            session_summary_cache: HashMap::new(),

            // Refresh & sync state
            last_synced: None,
            beads_last_check: Instant::now(),
            beads_last_mtime: None,

            // Mail network graph
            mail_network_state: None,
            mail_network_loading: false,
            mail_network_error: None,

            // Layout shaping
            layout_shaping_enabled: false,

            // Edge popup state
            edge_popup_open: None,

            // Session ingest
            ingest_loading: false,
            ingest_receiver: None,
            ingest_result: None,

            // Effective visible set
            effective_visible_nodes: HashSet::new(),
            effective_visible_count: 0,
            effective_visible_dirty: true,
            temporal_edges_dirty: false,
        };

        // Load initial data if connected
        if app.db_connected {
            app.load_graph();
        }

        app
    }

    fn reconnect_db(&mut self) {
        match DbClient::new() {
            Ok(client) => {
                self.db = Some(client);
                self.db_connected = true;
                self.db_error = None;
            }
            Err(e) => {
                self.db = None;
                self.db_connected = false;
                self.db_error = Some(e);
            }
        }
    }

    /// Mark settings as needing to be saved
    fn mark_settings_dirty(&mut self) {
        self.settings_dirty = true;
    }

    /// Copy current UI state to settings struct
    fn sync_settings_from_ui(&mut self) {
        self.settings.time_range_hours = self.time_range_hours;
        self.settings.node_size = self.node_size;
        self.settings.show_arrows = self.show_arrows;
        self.settings.timeline_enabled = self.timeline_enabled;
        self.settings.hover_scrubs_timeline = self.hover_scrubs_timeline;
        self.settings.color_mode = self.graph.color_mode;
        self.settings.importance_threshold = self.importance_threshold;
        self.settings.importance_filter = self.importance_filter;
        self.settings.tool_use_filter = self.tool_use_filter;
        self.settings.project_filter = self.project_filter;
        self.settings.sizing_preset = self.sizing_preset;
        self.settings.w_importance = self.w_importance;
        self.settings.w_tokens = self.w_tokens;
        self.settings.w_time = self.w_time;
        self.settings.max_node_multiplier = self.max_node_multiplier;
        self.settings.physics_enabled = self.graph.physics_enabled;
        self.settings.repulsion = self.layout.repulsion;
        self.settings.attraction = self.layout.attraction;
        self.settings.centering = self.layout.centering;
        self.settings.momentum = self.layout.momentum;
        self.settings.size_physics_weight = self.layout.size_physics_weight;
        self.settings.temporal_strength = self.layout.temporal_strength;
        self.settings.directed_stiffness = self.layout.directed_stiffness;
        self.settings.recency_centering = self.layout.recency_centering;
        self.settings.temporal_attraction_enabled = self.graph.temporal_attraction_enabled;
        self.settings.temporal_window_mins = (self.graph.temporal_window_secs / 60.0) as f32;
        self.settings.temporal_edge_opacity = self.temporal_edge_opacity;
        self.settings.max_temporal_edges = self.graph.max_temporal_edges;
        self.settings.proximity_edge_opacity = self.proximity_edge_opacity;
        self.settings.proximity_stiffness = self.proximity_stiffness;
        self.settings.proximity_delta = self.graph.score_proximity_delta;
        self.settings.proximity_strength = self.layout.similarity_strength;
        self.settings.max_proximity_edges = self.graph.max_proximity_edges;
        self.settings.max_neighbors_per_node = self.graph.max_neighbors_per_node;
        self.settings.beads_panel_open = self.beads_panel_open;
        self.settings.mail_panel_open = self.mail_panel_open;
        self.settings.histogram_panel_enabled = self.histogram_panel_enabled;
        self.settings.histogram_split_ratio = self.histogram_split_ratio;
        self.settings.sidebar_tab = self.sidebar_tab;
    }

    /// Copy settings values to UI fields (used when loading a preset)
    fn sync_ui_from_settings(&mut self) {
        // Don't sync time_range_hours since presets exclude data selection
        self.node_size = self.settings.node_size;
        self.show_arrows = self.settings.show_arrows;
        self.timeline_enabled = self.settings.timeline_enabled;
        self.hover_scrubs_timeline = self.settings.hover_scrubs_timeline;
        self.graph.color_mode = self.settings.color_mode;
        self.graph.timeline.speed = self.settings.timeline_speed;
        self.importance_threshold = self.settings.importance_threshold;
        self.importance_filter = self.settings.importance_filter;
        self.tool_use_filter = self.settings.tool_use_filter;
        self.project_filter = self.settings.project_filter;
        self.sizing_preset = self.settings.sizing_preset;
        self.w_importance = self.settings.w_importance;
        self.w_tokens = self.settings.w_tokens;
        self.w_time = self.settings.w_time;
        self.max_node_multiplier = self.settings.max_node_multiplier;
        self.graph.physics_enabled = self.settings.physics_enabled;
        self.layout.repulsion = self.settings.repulsion;
        self.layout.attraction = self.settings.attraction;
        self.layout.centering = self.settings.centering;
        self.layout.momentum = self.settings.momentum;
        self.layout.size_physics_weight = self.settings.size_physics_weight;
        self.layout.temporal_strength = self.settings.temporal_strength;
        self.layout.directed_stiffness = self.settings.directed_stiffness;
        self.layout.recency_centering = self.settings.recency_centering;
        self.graph.temporal_attraction_enabled = self.settings.temporal_attraction_enabled;
        self.graph.temporal_window_secs = (self.settings.temporal_window_mins * 60.0) as f64;
        self.temporal_edge_opacity = self.settings.temporal_edge_opacity;
        self.graph.max_temporal_edges = self.settings.max_temporal_edges;
        self.proximity_edge_opacity = self.settings.proximity_edge_opacity;
        self.proximity_stiffness = self.settings.proximity_stiffness;
        self.graph.score_proximity_delta = self.settings.proximity_delta;
        self.layout.similarity_strength = self.settings.proximity_strength;
        self.graph.max_proximity_edges = self.settings.max_proximity_edges;
        self.graph.max_neighbors_per_node = self.settings.max_neighbors_per_node;
        self.beads_panel_open = self.settings.beads_panel_open;
        self.mail_panel_open = self.settings.mail_panel_open;
        self.histogram_panel_enabled = self.settings.histogram_panel_enabled;
        self.histogram_split_ratio = self.settings.histogram_split_ratio;
        self.sidebar_tab = self.settings.sidebar_tab;
    }

    /// Save settings if dirty and enough time has passed (debounce)
    fn maybe_save_settings(&mut self) {
        if self.settings_dirty && self.last_settings_save.elapsed().as_secs() >= 2 {
            self.sync_settings_from_ui();
            self.settings.save();
            self.settings_dirty = false;
            self.last_settings_save = Instant::now();
        }
    }

    fn load_graph(&mut self) {
        let Some(ref db) = self.db else {
            self.db_error = Some("Database not connected".to_string());
            return;
        };

        self.loading = true;

        match db.fetch_graph(self.time_range_hours, None) {
            Ok(data) => {
                // Initialize with centered bounds
                let bounds = egui::Rect::from_center_size(
                    Pos2::new(400.0, 300.0),
                    Vec2::new(600.0, 400.0),
                );
                self.graph.load(data, bounds);
                self.loading = false;
                self.semantic_filter_cache = None;
                self.effective_visible_dirty = true;  // Invalidate cache

                // Extract available projects from nodes
                let projects: HashSet<String> = self.graph.data.nodes.iter()
                    .map(|n| n.project.clone())
                    .filter(|p| !p.is_empty())
                    .collect();
                let mut sorted_projects: Vec<String> = projects.into_iter().collect();
                sorted_projects.sort();
                self.available_projects = sorted_projects;
                // Select all projects by default
                self.selected_projects = self.available_projects.iter().cloned().collect();
                // Build hierarchical tree for project filter UI
                self.project_tree = Some(ProjectTreeNode::build(&self.available_projects));
                self.project_tree_expanded.clear();

                // Populate session metadata cache for histogram sorting
                self.session_metadata_cache.clear();
                for node in &self.graph.data.nodes {
                    let entry = self.session_metadata_cache
                        .entry(node.session_id.clone())
                        .or_insert((f64::MAX, 0));
                    if let Some(ts) = node.timestamp_secs() {
                        if ts < entry.0 { entry.0 = ts; }
                    }
                    entry.1 += 1;
                }

                // Fetch importance stats
                if let Ok(stats) = db.fetch_importance_stats() {
                    self.importance_stats = Some(ImportanceStats {
                        total_messages: stats.total_messages,
                        scored_messages: stats.scored_messages,
                    });
                }

                // Load semantic filters from API
                self.load_semantic_filters();

                // Load embedding stats
                self.load_embedding_stats();

                // Update last synced timestamp
                self.last_synced = Some(Instant::now());
            }
            Err(e) => {
                self.db_error = Some(e);
                self.loading = false;
            }
        }

        // Recompute bypass edges after data load (outside match to avoid borrow conflict)
        if !self.loading {
            self.recompute_bypass_edges();
        }
    }

    /// Check if .beads/ directory has changed since last check
    /// Returns true if changes detected and we should reload
    fn check_beads_changed(&mut self) -> bool {
        // Only check if auto-refresh is enabled
        if !self.settings.auto_refresh_enabled {
            return false;
        }

        // Check if enough time has passed since last check
        let now = Instant::now();
        let interval = std::time::Duration::from_secs_f32(self.settings.auto_refresh_interval_secs);
        if now.duration_since(self.beads_last_check) < interval {
            return false;
        }
        self.beads_last_check = now;

        // Try to get the modification time of the .beads/ directory
        // We look for a common file like the redirect or any files in the directory
        let beads_path = std::path::Path::new(".beads");
        if !beads_path.exists() {
            return false;
        }

        // Get the latest modification time from any file in .beads/
        let current_mtime = match std::fs::read_dir(beads_path) {
            Ok(entries) => {
                entries
                    .filter_map(|e| e.ok())
                    .filter_map(|e| e.metadata().ok())
                    .filter_map(|m| m.modified().ok())
                    .max()
            }
            Err(_) => None,
        };

        // If we can't get mtime, fall back to directory mtime
        let current_mtime = current_mtime.or_else(|| {
            std::fs::metadata(beads_path)
                .ok()
                .and_then(|m| m.modified().ok())
        });

        // Compare with previous
        let changed = match (current_mtime, self.beads_last_mtime) {
            (Some(current), Some(previous)) => current > previous,
            (Some(_), None) => true, // First check, consider changed to trigger initial state
            _ => false,
        };

        // Update stored mtime
        self.beads_last_mtime = current_mtime;

        changed
    }

    /// Load mail network data from API
    fn load_mail_network(&mut self) {
        self.mail_network_loading = true;
        self.mail_network_error = None;

        let api = ApiClient::new();
        match api.fetch_mail_network() {
            Ok(data) => {
                // Initialize state with positions in a circle
                let center = Pos2::new(125.0, 100.0);  // Center of mini-panel
                let radius = 60.0;
                self.mail_network_state = Some(MailNetworkState::new(data, center, radius));
                self.mail_network_loading = false;
            }
            Err(e) => {
                self.mail_network_error = Some(e);
                self.mail_network_loading = false;
            }
        }
    }

    fn update_fps(&mut self) {
        let now = Instant::now();
        let frame_time = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;

        self.frame_times.push(frame_time);
        if self.frame_times.len() > 60 {
            self.frame_times.remove(0);
        }

        if !self.frame_times.is_empty() {
            let avg_frame_time: f32 = self.frame_times.iter().sum::<f32>() / self.frame_times.len() as f32;
            self.fps = 1.0 / avg_frame_time;
        }
    }

    /// Find the node closest to the current scrubber position
    fn find_node_at_scrubber(&self) -> Option<crate::graph::types::GraphNode> {
        if self.graph.data.nodes.is_empty() {
            return None;
        }

        let scrubber_time = self.graph.timeline.time_at_position(self.graph.timeline.position);
        let mut closest_node: Option<&crate::graph::types::GraphNode> = None;
        let mut min_distance = f64::MAX;

        for node in &self.graph.data.nodes {
            if let Some(node_time) = node.timestamp_secs() {
                let distance = (scrubber_time - node_time).abs();
                if distance < min_distance {
                    min_distance = distance;
                    closest_node = Some(node);
                }
            }
        }

        closest_node.cloned()
    }

    /// Collect node IDs into inactive and filtered sets based on active filters.
    /// First-match-wins order: tool_use → importance → project.
    fn collect_filter_sets(&self) -> (HashSet<String>, HashSet<String>) {
        let mut inactive = HashSet::new();
        let mut filtered = HashSet::new();

        for node in &self.graph.data.nodes {
            // Tool use filter
            if self.tool_use_filter.is_active() && node.has_tool_usage {
                match self.tool_use_filter {
                    FilterMode::Inactive => { inactive.insert(node.id.clone()); }
                    FilterMode::Filtered => { filtered.insert(node.id.clone()); }
                    FilterMode::Off => {}
                }
                continue;
            }
            // Importance filter
            if self.importance_filter.is_active() {
                if let Some(score) = node.importance_score {
                    if score < self.importance_threshold {
                        match self.importance_filter {
                            FilterMode::Inactive => { inactive.insert(node.id.clone()); }
                            FilterMode::Filtered => { filtered.insert(node.id.clone()); }
                            FilterMode::Off => {}
                        }
                        continue;
                    }
                }
            }
            // Project filter
            if self.project_filter.is_active() && !self.selected_projects.contains(&node.project) {
                match self.project_filter {
                    FilterMode::Inactive => { inactive.insert(node.id.clone()); }
                    FilterMode::Filtered => { filtered.insert(node.id.clone()); }
                    FilterMode::Off => {}
                }
            }
        }

        (inactive, filtered)
    }

    /// Compute bypass edges that bridge over inactive nodes.
    /// Walks session chains (non-temporal, non-similarity edges),
    /// bridges over inactive_ids, stops at filtered_ids.
    fn compute_bypass_edges(&self, inactive: &HashSet<String>, filtered: &HashSet<String>) -> Vec<GraphEdge> {
        if inactive.is_empty() {
            return Vec::new();
        }

        // Build per-node successor map from session edges (non-temporal only)
        let mut next: HashMap<&str, &str> = HashMap::new();
        for edge in &self.graph.data.edges {
            if !edge.is_temporal && !edge.is_similarity {
                next.insert(&edge.source, &edge.target);
            }
        }

        let mut bypass = Vec::new();
        let mut seen_pairs: HashSet<(String, String)> = HashSet::new();

        for node in &self.graph.data.nodes {
            if inactive.contains(&node.id) || filtered.contains(&node.id) {
                continue; // Skip hidden nodes as sources
            }

            // Walk the chain from this node
            let mut cursor = node.id.as_str();
            while let Some(&successor) = next.get(cursor) {
                if filtered.contains(successor) {
                    break; // Stop at truly removed nodes
                }
                if !inactive.contains(successor) {
                    // Successor is visible — if we skipped any nodes, create bypass edge
                    if cursor != node.id.as_str() {
                        let pair = (node.id.clone(), successor.to_string());
                        if seen_pairs.insert(pair) {
                            bypass.push(GraphEdge {
                                source: node.id.clone(),
                                target: successor.to_string(),
                                session_id: node.session_id.clone(),
                                timestamp: None,
                                is_obsidian: false,
                                is_topic: false,
                                is_similarity: false,
                                is_temporal: false,
                                similarity: None,
                                query_index: None,
                            });
                        }
                    }
                    break;
                }
                cursor = successor;
            }
        }

        bypass
    }

    /// Convenience: collect filter sets and recompute bypass edges
    fn recompute_bypass_edges(&mut self) {
        let (inactive, filtered) = self.collect_filter_sets();
        self.bypass_edges = self.compute_bypass_edges(&inactive, &filtered);
    }

    /// Returns true if a node is hidden by any active filter (tool_use, importance, project).
    /// Does NOT check timeline or semantic filters (those are handled separately).
    fn is_node_hidden(&self, node: &crate::graph::types::GraphNode) -> bool {
        if self.tool_use_filter.is_active() && node.has_tool_usage {
            return true;
        }
        if self.importance_filter.is_active() {
            if let Some(score) = node.importance_score {
                if score < self.importance_threshold {
                    return true;
                }
            }
        }
        if self.project_filter.is_active() && !self.selected_projects.contains(&node.project) {
            return true;
        }
        false
    }

    /// Check if any semantic filters are active (not Off)
    fn has_active_semantic_filters(&self) -> bool {
        self.semantic_filter_modes.values()
            .any(|mode| *mode != SemanticFilterMode::Off)
    }

    /// Build adjacency list from graph edges for BFS neighbor lookup
    fn build_adjacency_list(&self, include_temporal: bool) -> HashMap<String, Vec<String>> {
        build_adjacency_list(&self.graph.data.edges, include_temporal)
    }

    /// Expand a set of nodes to include neighbors up to given depth using BFS
    fn expand_to_neighbors(&self, seeds: &HashSet<String>, depth: usize, adj: &HashMap<String, Vec<String>>) -> HashSet<String> {
        expand_to_neighbors(seeds, depth, adj)
    }

    /// Compute the set of nodes visible based on semantic filters
    /// Returns None if no semantic filters are active
    /// Returns Some(HashSet) with visible node IDs when filters are active
    fn compute_semantic_filter_visible_set(&self) -> Option<HashSet<String>> {
        if !self.has_active_semantic_filters() {
            return None;
        }

        // Build adjacency list once for BFS (always include temporal for semantic filters)
        let adj = self.build_adjacency_list(true);

        // Collect include filters (union/OR) and exclude filters separately.
        // Multiple includes mean "match ANY include filter" (union), not all.
        let mut include_union: HashSet<String> = HashSet::new();
        let mut has_includes = false;

        for (filter_id, mode) in &self.semantic_filter_modes {
            match mode {
                SemanticFilterMode::Include => {
                    has_includes = true;
                    for node in &self.graph.data.nodes {
                        if node.semantic_filter_matches.contains(filter_id) {
                            include_union.insert(node.id.clone());
                        }
                    }
                }
                SemanticFilterMode::IncludePlus1 => {
                    has_includes = true;
                    let matching: HashSet<String> = self.graph.data.nodes.iter()
                        .filter(|n| n.semantic_filter_matches.contains(filter_id))
                        .map(|n| n.id.clone())
                        .collect();
                    let expanded = self.expand_to_neighbors(&matching, 1, &adj);
                    include_union.extend(expanded);
                }
                SemanticFilterMode::IncludePlus2 => {
                    has_includes = true;
                    let matching: HashSet<String> = self.graph.data.nodes.iter()
                        .filter(|n| n.semantic_filter_matches.contains(filter_id))
                        .map(|n| n.id.clone())
                        .collect();
                    let expanded = self.expand_to_neighbors(&matching, 2, &adj);
                    include_union.extend(expanded);
                }
                _ => {}
            }
        }

        // Debug: log filter state
        eprintln!("[semantic-filter] has_includes={}, include_union.len={}, total_nodes={}, modes: {:?}",
            has_includes, include_union.len(), self.graph.data.nodes.len(),
            self.semantic_filter_modes.iter().map(|(id, m)| (*id, *m)).collect::<Vec<_>>());
        // Debug: check how many nodes have any semantic_filter_matches at all
        let nodes_with_matches = self.graph.data.nodes.iter().filter(|n| !n.semantic_filter_matches.is_empty()).count();
        eprintln!("[semantic-filter] nodes with any filter matches: {}", nodes_with_matches);

        // Start with include union (or all nodes if no includes)
        let mut visible = if has_includes {
            include_union
        } else {
            self.graph.data.nodes.iter().map(|n| n.id.clone()).collect()
        };

        // Then apply exclude filters (remove matching nodes)
        for (filter_id, mode) in &self.semantic_filter_modes {
            if *mode == SemanticFilterMode::Exclude {
                for node in &self.graph.data.nodes {
                    if node.semantic_filter_matches.contains(filter_id) {
                        visible.remove(&node.id);
                    }
                }
            }
        }

        eprintln!("[semantic-filter] final visible: {}", visible.len());
        Some(visible)
    }

    /// Check if a node passes the semantic filter criteria
    /// Returns true if the node should be visible, false if it should be hidden
    fn node_passes_semantic_filters(&self, node: &crate::graph::types::GraphNode) -> bool {
        // For simple Include/Exclude, do quick per-node check
        // For +1/+2 modes, we need the pre-computed set (caller should use compute_semantic_filter_visible_set)

        // Check excludes first — any exclude match hides the node
        for (filter_id, mode) in &self.semantic_filter_modes {
            if *mode == SemanticFilterMode::Exclude && node.semantic_filter_matches.contains(filter_id) {
                return false;
            }
        }

        // Check includes — node must match ANY active include filter (OR logic)
        let has_includes = self.semantic_filter_modes.iter()
            .any(|(_, mode)| *mode == SemanticFilterMode::Include);

        if has_includes {
            let matches_any = self.semantic_filter_modes.iter()
                .any(|(filter_id, mode)| {
                    *mode == SemanticFilterMode::Include && node.semantic_filter_matches.contains(filter_id)
                });
            if !matches_any {
                return false;
            }
        }

        true
    }

    /// Check if any semantic filter uses expansion modes (+1 or +2)
    fn has_expansion_semantic_filters(&self) -> bool {
        self.semantic_filter_modes.values()
            .any(|mode| matches!(mode, SemanticFilterMode::IncludePlus1 | SemanticFilterMode::IncludePlus2))
    }

    /// Check if a node is a "same-project future node" (outside timeline but same project as selected/hovered)
    fn is_same_project_future_node(&self, node: &crate::graph::types::GraphNode) -> bool {
        // Must be outside timeline window (future)
        if self.graph.is_node_visible(&node.id) {
            return false;
        }

        // Check if same project as selected or hovered node
        let reference_project = self.graph.selected_node.as_ref()
            .or(self.graph.hovered_node.as_ref())
            .and_then(|id| self.graph.get_node(id))
            .map(|n| &n.project);

        if let Some(ref_project) = reference_project {
            return &node.project == ref_project;
        }

        false
    }

    /// Debug: Check if node is after the current playhead position
    fn is_after_playhead(&self, node: &crate::graph::types::GraphNode) -> bool {
        if !self.timeline_enabled {
            return false;
        }
        let scrubber_time = self.graph.timeline.time_at_position(self.graph.timeline.position);
        if let Some(node_time) = node.timestamp_secs() {
            node_time > scrubber_time
        } else {
            false
        }
    }

    /// Debug: Check if node is in same session as selected node
    fn is_same_session_as_selected(&self, node: &crate::graph::types::GraphNode) -> bool {
        if let Some(ref selected_id) = self.graph.selected_node {
            if let Some(selected_node) = self.graph.get_node(selected_id) {
                return node.session_id == selected_node.session_id;
            }
        }
        false
    }

    /// Debug: Check if node is in same project as selected node
    fn is_same_project_as_selected(&self, node: &crate::graph::types::GraphNode) -> bool {
        if let Some(ref selected_id) = self.graph.selected_node {
            if let Some(selected_node) = self.graph.get_node(selected_id) {
                return node.project == selected_node.project;
            }
        }
        false
    }

    /// Compute which nodes should participate in physics simulation.
    /// Uses the effective visible set + adds same-project future nodes.
    /// Returns None if no filtering is active (simulate all nodes).
    fn compute_physics_visible_nodes(&self) -> Option<HashSet<String>> {
        if !self.any_filter_active() && !self.any_proximity_active() {
            return None;
        }

        // Clone effective visible set and add same-project future nodes
        let mut visible = self.effective_visible_nodes.clone();
        for node in &self.graph.data.nodes {
            if self.is_same_project_future_node(node) {
                visible.insert(node.id.clone());
            }
        }

        Some(visible)
    }

    /// Compute node sizes for physics simulation
    /// Returns None if size_physics_weight is 0 (uniform masses)
    /// Returns Some(HashMap) with node_id -> size when physics uses variable mass
    fn compute_node_sizes(&self) -> Option<std::collections::HashMap<String, f32>> {
        // If weight is ~0, return None for uniform masses (optimization)
        if self.layout.size_physics_weight < 0.001 {
            return None;
        }

        let mut sizes = std::collections::HashMap::new();

        for node in &self.graph.data.nodes {
            // Same formula as visual sizing, but for ALL nodes
            // (physics may simulate nodes not currently drawn)

            // 1. Importance factor (0-1, default 0.5)
            let importance = node.importance_score.unwrap_or(0.5);
            let imp_factor = (self.w_importance * importance).exp();

            // 2. Token factor (log-normalized 0-1)
            let tokens_norm = self.graph.normalize_tokens(node);
            let tok_factor = (self.w_tokens * tokens_norm).exp();

            // 3. Time/recency factor (distance from scrubber, 0-1)
            let time_factor = if self.graph.timeline.max_time > self.graph.timeline.min_time {
                if let Some(node_time) = node.timestamp_secs() {
                    let time_range = self.graph.timeline.max_time - self.graph.timeline.min_time;
                    let scrubber_time = self.graph.timeline.time_at_position(self.graph.timeline.position);
                    let distance = (scrubber_time - node_time).abs();
                    let normalized_distance = (distance / time_range).clamp(0.0, 1.0) as f32;
                    (-self.w_time * normalized_distance).exp()
                } else {
                    1.0
                }
            } else {
                1.0
            };

            // Combine factors multiplicatively (same as visual sizing)
            let size = imp_factor * tok_factor * time_factor;
            sizes.insert(node.id.clone(), size);
        }

        Some(sizes)
    }

    /// Trigger summary fetch for a double-clicked node
    fn trigger_summary_for_node(&mut self, node_id: String) {
        if let Some(node) = self.graph.get_node(&node_id) {
            let session_id = node.session_id.clone();
            let timestamp = node.timestamp.clone();

            // Store state
            self.summary_node_id = Some(node_id);
            self.summary_session_id = Some(session_id.clone());
            self.summary_timestamp = timestamp.clone();
            self.summary_loading = true; // Enable point-in-time summary via API
            self.summary_data = None;
            self.summary_error = None;

            // Reset session summary state
            self.session_summary_data = None;
            self.session_summary_loading = true;

            // Open floating window
            if !self.summary_window_dragged {
                self.summary_window_dragged = false;
            }
            self.summary_window_open = true;

            // Create channels for both summaries
            let (partial_tx, partial_rx) = mpsc::channel();
            let (session_tx, session_rx) = mpsc::channel();
            self.summary_receiver = Some(partial_rx);
            self.session_summary_receiver = Some(session_rx);

            // Fetch point-in-time summary via API (with AI generation)
            if let Some(ref ts) = timestamp {
                let api = ApiClient::new();
                let sid = session_id.clone();
                let ts_clone = ts.clone();
                std::thread::spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        api.fetch_partial_summary(&sid, &ts_clone)
                    }));
                    match result {
                        Ok(r) => { let _ = partial_tx.send(r); }
                        Err(_) => { let _ = partial_tx.send(Err("Thread panicked".to_string())); }
                    }
                });
            } else {
                // No timestamp - can't do point-in-time summary
                self.summary_loading = false;
            }

            // Fetch session summary via API (with generate_if_missing=true)
            let api = ApiClient::new();
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    api.fetch_session_summary(&session_id, true)
                }));
                match result {
                    Ok(r) => { let _ = session_tx.send(r); }
                    Err(_) => { let _ = session_tx.send(Err("Thread panicked".to_string())); }
                }
            });
        }
    }

    /// Trigger neighborhood summary for a Ctrl+Clicked node and its direct neighbors
    fn trigger_neighborhood_summary(&mut self, node_id: String) {
        // Build adjacency list and find neighbors at configured depth
        let adj = self.build_adjacency_list(self.neighborhood_include_temporal);
        let mut seeds = HashSet::new();
        seeds.insert(node_id.clone());
        let neighbor_ids = self.expand_to_neighbors(&seeds, self.neighborhood_depth, &adj);

        let message_ids: Vec<String> = neighbor_ids.into_iter().collect();
        let count = message_ids.len();

        // Set loading state
        self.neighborhood_summary_data = None;
        self.neighborhood_summary_loading = true;
        self.neighborhood_summary_error = None;
        self.neighborhood_summary_center_node = Some(node_id);
        self.neighborhood_summary_count = count;

        // Open neighborhood window
        if !self.neighborhood_window_dragged {
            self.neighborhood_window_dragged = false;
        }
        self.neighborhood_window_open = true;

        // Create channel and spawn background thread
        let (tx, rx) = mpsc::channel();
        self.neighborhood_summary_receiver = Some(rx);

        let api = ApiClient::new();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                api.fetch_neighborhood_summary(message_ids)
            }));
            match result {
                Ok(r) => { let _ = tx.send(r); }
                Err(_) => { let _ = tx.send(Err("Thread panicked".to_string())); }
            }
        });
    }

    /// Load semantic filters from the API
    fn load_semantic_filters(&mut self) {
        self.semantic_filter_loading = true;

        let api = ApiClient::new();
        match api.fetch_semantic_filters() {
            Ok(filters) => {
                self.semantic_filters = filters;
                self.semantic_filter_loading = false;
            }
            Err(e) => {
                eprintln!("Failed to load semantic filters: {}", e);
                self.semantic_filter_loading = false;
            }
        }
    }

    /// Create a new semantic filter (LLM-scored)
    fn create_semantic_filter(&mut self) {
        let name = self.new_filter_input.trim().to_string();
        if name.is_empty() {
            return;
        }

        let api = ApiClient::new();
        match api.create_semantic_filter(&name, &name, "semantic") {
            Ok(filter) => {
                self.semantic_filters.push(filter);
                self.new_filter_input.clear();
            }
            Err(e) => {
                eprintln!("Failed to create semantic filter: {}", e);
            }
        }
    }

    /// Create a rule-based filter (auto-scored, no LLM)
    fn create_rule_filter(&mut self, name: &str, query_text: &str) {
        let api = ApiClient::new();
        match api.create_semantic_filter(name, query_text, "rule") {
            Ok(filter) => {
                self.semantic_filters.push(filter);
                self.semantic_filter_cache = None;
                // Reload graph so nodes pick up new semantic_filter_matches
                self.load_graph();
            }
            Err(e) => {
                eprintln!("Failed to create rule filter: {}", e);
            }
        }
    }

    /// Delete a semantic filter
    fn delete_semantic_filter(&mut self, filter_id: i32) {
        let api = ApiClient::new();
        match api.delete_semantic_filter(filter_id) {
            Ok(()) => {
                self.semantic_filters.retain(|f| f.id != filter_id);
                self.semantic_filter_modes.remove(&filter_id);
            }
            Err(e) => {
                eprintln!("Failed to delete semantic filter: {}", e);
            }
        }
    }

    /// Trigger categorization for a semantic filter (runs in background)
    fn trigger_categorization(&mut self, filter_id: i32) {
        self.categorizing_filter_id = Some(filter_id);
        self.categorization_progress = None;

        // Run categorization in background thread
        let (tx, rx) = mpsc::channel();

        std::thread::spawn(move || {
            let api = ApiClient::new();
            let result = api.trigger_categorization(filter_id);
            let _ = tx.send(result);
        });

        // Poll progress in a separate thread
        let (progress_tx, progress_rx) = mpsc::channel();
        let done_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done_flag_clone = done_flag.clone();

        std::thread::spawn(move || {
            let api = ApiClient::new();
            while !done_flag_clone.load(std::sync::atomic::Ordering::Relaxed) {
                if let Ok(status) = api.fetch_filter_status(filter_id) {
                    if progress_tx.send(status).is_err() {
                        break; // receiver dropped
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        });

        // Store the done flag so we can stop polling when categorization finishes
        self.categorization_done_flag = Some(done_flag);
        self.categorization_receiver = Some(rx);
        self.categorization_progress_rx = Some(progress_rx);
    }

    /// Check if any proximity query is active (has results)
    fn any_proximity_active(&self) -> bool {
        self.proximity_queries.iter().any(|q| q.active)
    }

    /// Get a reference to the effective visible set for passing to graph methods.
    /// Returns Some(&HashSet) when any filter is active, None otherwise.
    fn effective_visible_set(&self) -> Option<&HashSet<String>> {
        if self.any_filter_active() {
            Some(&self.effective_visible_nodes)
        } else {
            None
        }
    }

    /// Check if any filter is active that would reduce the visible set
    fn any_filter_active(&self) -> bool {
        self.timeline_enabled
            || self.importance_filter.is_active()
            || self.project_filter.is_active()
            || self.has_active_semantic_filters()
            || self.tool_use_filter.is_active()
            || self.histogram_session_filter.is_some()
    }

    /// Check if a single node passes ALL active filters.
    /// Used by rebuild_effective_visible_set() to build the unified set.
    fn is_node_effectively_visible(&self, node: &crate::graph::types::GraphNode, semantic_visible: &Option<HashSet<String>>) -> bool {
        // Timeline filter
        if self.timeline_enabled && !self.graph.timeline.visible_nodes.contains(&node.id) {
            return false;
        }
        // Importance filter
        if self.importance_filter.is_active() {
            if let Some(score) = node.importance_score {
                if score < self.importance_threshold {
                    return false;
                }
            }
        }
        // Project filter
        if self.project_filter.is_active() && !self.selected_projects.contains(&node.project) {
            return false;
        }
        // Session filter (histogram drill-down)
        if let Some(ref sf) = self.histogram_session_filter {
            if node.session_id != *sf {
                return false;
            }
        }
        // Semantic filter
        if let Some(ref sem_visible) = semantic_visible {
            if !sem_visible.contains(&node.id) {
                return false;
            }
        }
        // Tool use filter
        if self.tool_use_filter.is_active() && node.has_tool_usage {
            return false;
        }
        true
    }

    /// Rebuild the effective visible set by iterating all nodes once.
    /// Should be called when effective_visible_dirty is true.
    fn rebuild_effective_visible_set(&mut self) {
        // Ensure semantic filter cache is fresh
        if self.semantic_filter_cache.is_none() && self.has_active_semantic_filters() {
            self.semantic_filter_cache = self.compute_semantic_filter_visible_set();
        }
        let semantic_visible = self.semantic_filter_cache.clone();

        self.effective_visible_nodes.clear();
        for node in &self.graph.data.nodes {
            if self.is_node_effectively_visible(node, &semantic_visible) {
                self.effective_visible_nodes.insert(node.id.clone());
            }
        }
        self.effective_visible_count = self.effective_visible_nodes.len();
        self.effective_visible_dirty = false;
    }

    /// Check if any proximity query is currently loading
    fn any_proximity_loading(&self) -> bool {
        self.proximity_queries.iter().any(|q| q.loading)
    }

    /// Total edge count across all proximity queries
    fn total_proximity_edge_count(&self) -> usize {
        self.proximity_queries.iter().map(|q| q.edge_count).sum()
    }

    /// Add a new proximity query, dedup, assign color, and trigger fetch
    fn add_proximity_query(&mut self, text: String) {
        let text = text.trim().to_string();
        if text.is_empty() { return; }
        // Dedup check
        if self.proximity_queries.iter().any(|q| q.query == text) { return; }

        let color = QUERY_COLORS[self.proximity_queries.len() % QUERY_COLORS.len()];
        let idx = self.proximity_queries.len();
        self.proximity_queries.push(ProximityQuery {
            query: text,
            color,
            scores: HashMap::new(),
            edges: Vec::new(),
            edge_count: 0,
            active: false,
            loading: false,
            rx: None,
        });
        self.graph.score_proximity_enabled = true;
        self.trigger_proximity_fetch_for(idx);
    }

    /// Remove a proximity query by index, rebuild edges
    fn remove_proximity_query(&mut self, index: usize) {
        if index >= self.proximity_queries.len() { return; }
        self.proximity_queries.remove(index);
        // Fix heat map index
        if let Some(hmi) = self.proximity_heat_map_index {
            if hmi == index {
                self.proximity_heat_map_index = None;
            } else if hmi > index {
                self.proximity_heat_map_index = Some(hmi - 1);
            }
        }
        self.rebuild_all_proximity_edges();
        if self.proximity_queries.is_empty() {
            self.graph.score_proximity_enabled = false;
        }
    }

    /// Combine all per-query edge vecs into graph.data.edges
    fn rebuild_all_proximity_edges(&mut self) {
        let mut all_edges = Vec::new();
        for (idx, q) in self.proximity_queries.iter().enumerate() {
            for edge in &q.edges {
                let mut e = edge.clone();
                e.query_index = Some(idx);
                all_edges.push(e);
            }
        }
        self.graph.set_proximity_edges(all_edges);
    }

    /// Trigger proximity fetch for a specific query index
    fn trigger_proximity_fetch_for(&mut self, index: usize) {
        let query = self.proximity_queries[index].query.trim().to_string();
        if query.is_empty() { return; }

        self.proximity_queries[index].loading = true;

        let delta = self.graph.score_proximity_delta;
        let max_edges = self.graph.max_proximity_edges;
        let max_neighbors = self.graph.max_neighbors_per_node;

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let api = ApiClient::new();
            match api.fetch_proximity_edges(&query, delta, max_edges, max_neighbors) {
                Ok(response) => {
                    let edges: Vec<GraphEdge> = response.edges.into_iter().map(|e| {
                        GraphEdge::similarity(e.source, e.target, e.strength, None)
                    }).collect();
                    let _ = tx.send(Ok((edges, response.scores)));
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            }
        });

        self.proximity_queries[index].rx = Some(rx);
    }

    /// Re-fetch all active queries (e.g., after delta/max change)
    fn refetch_all_proximity_queries(&mut self) {
        for i in 0..self.proximity_queries.len() {
            if self.proximity_queries[i].active || !self.proximity_queries[i].query.is_empty() {
                self.trigger_proximity_fetch_for(i);
            }
        }
    }

    /// Clear all proximity queries, edges, and overlay
    fn clear_proximity(&mut self) {
        self.proximity_queries.clear();
        self.proximity_input.clear();
        self.proximity_edge_count_filtered = 0;
        self.proximity_heat_map_index = None;
        self.graph.set_proximity_edges(Vec::new());
        self.graph.score_proximity_enabled = false;
    }

    /// Load embedding stats from the API
    fn load_embedding_stats(&mut self) {
        let api = ApiClient::new();
        match api.fetch_embedding_stats() {
            Ok(stats) => {
                self.embedding_stats = Some(stats);
            }
            Err(e) => {
                eprintln!("Failed to load embedding stats: {}", e);
            }
        }
    }

    /// Trigger embedding generation (runs in background)
    fn trigger_embedding_generation(&mut self) {
        self.embedding_gen_loading = true;

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let api = ApiClient::new();
            let result = api.generate_embeddings(1000);
            let _ = tx.send(result);
        });

        self.embedding_gen_receiver = Some(rx);
    }

    /// Trigger embedding generation for visible nodes only (runs in background)
    fn trigger_embedding_generation_visible(&mut self) {
        // Collect IDs of currently visible nodes
        let visible_ids: Vec<String> = self.get_visible_node_ids();
        if visible_ids.is_empty() {
            return;
        }

        self.embedding_gen_loading = true;

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let api = ApiClient::new();
            let result = api.generate_embeddings_visible(&visible_ids);
            let _ = tx.send(result);
        });

        self.embedding_gen_receiver = Some(rx);
    }

    /// Get IDs of all currently visible (non-filtered) nodes
    fn get_visible_node_ids(&self) -> Vec<String> {
        self.effective_visible_nodes.iter().cloned().collect()
    }

    /// Start rescoring importance for visible nodes (runs in background with progress)
    fn start_rescore_visible(&mut self) {
        // Collect unique session IDs from visible nodes
        let session_ids: Vec<String> = self.get_visible_session_ids();

        if session_ids.is_empty() {
            return;
        }

        self.rescore_loading = true;
        self.rescore_result = None;
        self.rescore_progress = None;

        // Run rescore in background thread with streaming progress
        let (tx, rx) = mpsc::channel();

        std::thread::spawn(move || {
            let api = ApiClient::new();
            if let Err(e) = api.rescore_importance_stream(session_ids, tx.clone()) {
                let _ = tx.send(RescoreEvent::Error(e));
            }
        });

        self.rescore_receiver = Some(rx);
    }

    /// Start re-ingestion of Claude sessions in background thread.
    /// Computes the since window from the most recent node timestamp.
    fn start_ingest(&mut self) {
        self.ingest_loading = true;
        self.ingest_result = None;

        // Find the most recent node timestamp to determine staleness
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        let max_ts = self.graph.data.nodes.iter()
            .filter_map(|n| n.timestamp_secs())
            .fold(0.0_f64, f64::max);

        let hours_stale = if max_ts > 0.0 {
            ((now - max_ts) / 3600.0).ceil() as u64
        } else {
            24 // fallback if no nodes loaded
        };

        // Convert to a --since string, minimum 1h, add 1h padding
        let since = if hours_stale + 1 >= 48 {
            format!("{}d", (hours_stale + 1 + 23) / 24) // round up to days
        } else {
            format!("{}h", (hours_stale + 1).max(1))
        };

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let api = ApiClient::new();
            let _ = tx.send(api.trigger_ingest(&since));
        });

        self.ingest_receiver = Some(rx);
    }

    /// Get unique session IDs from currently visible nodes
    fn get_visible_session_ids(&self) -> Vec<String> {
        let mut session_ids: HashSet<String> = HashSet::new();
        for node in &self.graph.data.nodes {
            if self.effective_visible_nodes.contains(&node.id) {
                session_ids.insert(node.session_id.clone());
            }
        }
        session_ids.into_iter().collect()
    }

    /// Render the floating summary window (Point-in-Time + Session Summary, triggered by double-click)
    fn render_summary_window(&mut self, ctx: &egui::Context) {
        if !self.summary_window_open {
            return;
        }

        // Calculate default position (top-right, offset from edge)
        let screen_rect = ctx.screen_rect();
        let default_pos = egui::pos2(screen_rect.right() - 420.0, 60.0);

        let mut open = self.summary_window_open;

        let window_response = egui::Window::new("Summary")
            .open(&mut open)
            .default_pos(default_pos)
            .default_size([400.0, 500.0])
            .resizable(true)
            .collapsible(true)
            .show(ctx, |ui| {
                // Resume session button bar
                if let Some(ref session_id) = self.summary_session_id.clone() {
                    let project = self.summary_node_id.as_ref()
                        .and_then(|nid| self.graph.get_node(nid))
                        .map(|n| n.project.clone())
                        .unwrap_or_default();

                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("Session: {}…", &session_id[..8.min(session_id.len())]))
                            .small()
                            .color(egui::Color32::GRAY));

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let recently_copied = self.resume_copied_at
                                .map(|t| t.elapsed().as_millis() < 1500)
                                .unwrap_or(false);

                            let btn_text = if recently_copied { "Copied!" } else { "Resume" };
                            let btn = ui.button(btn_text);

                            if recently_copied {
                                ctx.request_repaint();
                            }

                            if btn.clicked() {
                                // Expand ~/ to full home path
                                let full_path = if project.starts_with("~/") {
                                    if let Some(home) = dirs::home_dir() {
                                        format!("{}{}", home.display(), &project[1..])
                                    } else {
                                        project.clone()
                                    }
                                } else {
                                    project.clone()
                                };

                                let cmd = format!("cd {} && claude --resume {}", full_path, session_id);
                                ui.output_mut(|o| o.copied_text = cmd);
                                self.resume_copied_at = Some(Instant::now());
                            }

                            btn.on_hover_text("Copy zsh command to resume this Claude Code session");
                        });
                    });
                    ui.separator();
                }

                // Point-in-Time Summary section
                egui::CollapsingHeader::new("Point-in-Time Summary")
                    .default_open(true)
                    .show(ui, |ui| {
                        if self.summary_loading {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label("Generating summary...");
                            });
                            ui.add_space(8.0);
                            // Skeleton preview of content
                            theme::skeleton_lines(ui, 3, ui.available_width() * 0.9);
                        } else if let Some(ref error) = self.summary_error.clone() {
                            ui.colored_label(theme::state::ERROR, format!("Error: {}", error));
                            if ui.button("Dismiss").clicked() {
                                self.summary_error = None;
                                self.summary_node_id = None;
                            }
                        } else if let Some(ref data) = self.summary_data.clone() {
                            // Message counts
                            ui.horizontal(|ui| {
                                ui.label(format!("Messages: {} you / {} Claude",
                                    data.user_count, data.assistant_count));
                            });
                            ui.add_space(5.0);

                            // Summary
                            ui.label(egui::RichText::new("Summary").strong());
                            egui::ScrollArea::vertical()
                                .max_height(80.0)
                                .id_salt("window_summary_scroll")
                                .show(ui, |ui| {
                                    ui.label(&data.summary);
                                });
                            ui.add_space(5.0);

                            // Current Focus
                            if !data.current_focus.is_empty() {
                                ui.label(egui::RichText::new("Working On").strong());
                                ui.label(&data.current_focus);
                                ui.add_space(5.0);
                            }

                            // Completed Work
                            if !data.completed_work.is_empty() {
                                ui.label(egui::RichText::new("Completed").strong().color(theme::state::SUCCESS));
                                egui::ScrollArea::vertical()
                                    .max_height(60.0)
                                    .id_salt("window_completed_scroll")
                                    .show(ui, |ui| {
                                        for line in data.completed_work.split('\n').filter(|l| !l.is_empty()) {
                                            ui.label(line);
                                        }
                                    });
                                ui.add_space(5.0);
                            }

                            // Unsuccessful Attempts
                            if !data.unsuccessful_attempts.is_empty() {
                                ui.label(egui::RichText::new("Tried but Failed").strong().color(Color32::from_rgb(255, 100, 100)));
                                egui::ScrollArea::vertical()
                                    .max_height(60.0)
                                    .id_salt("window_failed_scroll")
                                    .show(ui, |ui| {
                                        for line in data.unsuccessful_attempts.split('\n').filter(|l| !l.is_empty()) {
                                            ui.label(line);
                                        }
                                    });
                            }

                            // Importance grading for the clicked node
                            if let Some(ref node_id) = self.summary_node_id.clone() {
                                if let Some(node) = self.graph.get_node(&node_id) {
                                    if node.importance_score.is_some() || node.importance_reason.is_some() {
                                        ui.add_space(5.0);
                                        ui.label(egui::RichText::new("Importance").strong().color(Color32::from_rgb(255, 193, 7)));
                                        if let Some(score) = node.importance_score {
                                            ui.label(format!("Score: {:.0}%", score * 100.0));
                                        }
                                        if let Some(ref reason) = node.importance_reason {
                                            ui.label(reason);
                                        }
                                    }
                                }
                            }
                        } else {
                            ui.colored_label(egui::Color32::GRAY, "(Disabled - requires AI generation)");
                        }
                    });

                ui.separator();

                // Session Summary section
                egui::CollapsingHeader::new("Session Summary")
                    .default_open(true)
                    .show(ui, |ui| {
                        if self.session_summary_loading {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label("Generating summary...");
                            });
                            ui.add_space(8.0);
                            // Skeleton preview of summary content
                            theme::skeleton_lines(ui, 4, ui.available_width() * 0.9);
                        } else if let Some(ref data) = self.session_summary_data.clone() {
                            // Check for errors first
                            if let Some(ref err) = data.error {
                                ui.colored_label(theme::state::ERROR, format!("Error: {}", err));
                            } else if !data.exists {
                                ui.label("No summary in database for this session.");
                            } else {
                                // Show "just generated" badge if applicable
                                if data.generated {
                                    ui.colored_label(theme::state::SUCCESS, "✓ Just generated");
                                    ui.add_space(5.0);
                                }

                                // Project and topics
                                if let Some(ref project) = data.detected_project {
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new("Project:").strong());
                                        ui.label(project);
                                    });
                                }

                                if let Some(ref topics) = data.topics {
                                    if !topics.is_empty() {
                                        ui.horizontal_wrapped(|ui| {
                                            ui.label(egui::RichText::new("Topics:").strong());
                                            for topic in topics.iter().take(5) {
                                                ui.label(format!("[{}]", topic));
                                            }
                                        });
                                    }
                                }

                                ui.add_space(5.0);

                                // Summary paragraph
                                if let Some(ref summary) = data.summary {
                                    ui.label(egui::RichText::new("Summary").strong());
                                    egui::ScrollArea::vertical()
                                        .max_height(100.0)
                                        .id_salt("window_session_summary_scroll")
                                        .show(ui, |ui| {
                                            ui.label(summary);
                                        });
                                    ui.add_space(5.0);
                                }

                                // Completed work
                                if let Some(ref completed) = data.completed_work {
                                    if !completed.is_empty() {
                                        ui.label(egui::RichText::new("Completed Work").strong().color(theme::state::SUCCESS));
                                        egui::ScrollArea::vertical()
                                            .max_height(80.0)
                                            .id_salt("window_session_completed_scroll")
                                            .show(ui, |ui| {
                                                for line in completed.split('\n').filter(|l| !l.is_empty()) {
                                                    ui.label(line);
                                                }
                                            });
                                        ui.add_space(5.0);
                                    }
                                }

                                // User requests
                                if let Some(ref requests) = data.user_requests {
                                    if !requests.is_empty() {
                                        ui.label(egui::RichText::new("User Requests").strong().color(Color32::from_rgb(100, 149, 237)));
                                        egui::ScrollArea::vertical()
                                            .max_height(60.0)
                                            .id_salt("window_session_requests_scroll")
                                            .show(ui, |ui| {
                                                for line in requests.split('\n').filter(|l| !l.is_empty()) {
                                                    ui.label(line);
                                                }
                                            });
                                    }
                                }
                            }
                        } else {
                            ui.label("Double-click a node to see");
                            ui.label("the full session summary.");
                        }
                    });

                ui.add_space(10.0);
                ui.separator();

                // Clear button - only clears summary window state
                if ui.button("Clear & Close").clicked() {
                    self.summary_data = None;
                    self.summary_node_id = None;
                    self.session_summary_data = None;
                    self.summary_window_open = false;
                    self.summary_window_dragged = false;
                }
            });

        self.summary_window_open = open;

        // Detect if window was dragged from default position
        if let Some(inner) = window_response {
            let pos = inner.response.rect.left_top();
            let dist = (pos - default_pos).length();
            if dist > 10.0 {
                self.summary_window_dragged = true;
            }
        }
    }

    /// Render the floating neighborhood window (Neighborhood Summary, triggered by Ctrl+Click)
    fn render_neighborhood_window(&mut self, ctx: &egui::Context) {
        if !self.neighborhood_window_open {
            return;
        }

        // Calculate default position (offset from summary window position)
        let screen_rect = ctx.screen_rect();
        let default_pos = egui::pos2(screen_rect.right() - 420.0, 580.0);

        let mut open = self.neighborhood_window_open;

        let window_response = egui::Window::new("Neighborhood")
            .open(&mut open)
            .default_pos(default_pos)
            .default_size([400.0, 350.0])
            .resizable(true)
            .collapsible(true)
            .show(ctx, |ui| {
                // Depth slider
                ui.horizontal(|ui| {
                    ui.label("Depth:");
                    ui.add(egui::Slider::new(&mut self.neighborhood_depth, 1..=5).text("edges"));
                });

                // Temporal edge toggle
                ui.checkbox(&mut self.neighborhood_include_temporal, "Include temporal edges");

                ui.add_space(5.0);

                if self.neighborhood_summary_loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(format!("Summarizing {} nodes...", self.neighborhood_summary_count));
                    });
                    ui.add_space(8.0);
                    theme::skeleton_lines(ui, 3, ui.available_width() * 0.9);
                } else if let Some(ref error) = self.neighborhood_summary_error.clone() {
                    ui.colored_label(theme::state::ERROR, format!("Error: {}", error));
                } else if let Some(ref data) = self.neighborhood_summary_data.clone() {
                    // Node/session counts
                    ui.horizontal(|ui| {
                        ui.label(format!("{} nodes across {} sessions",
                            data.node_count, data.session_count));
                    });
                    ui.add_space(5.0);

                    // Summary
                    ui.label(egui::RichText::new("Summary").strong());
                    egui::ScrollArea::vertical()
                        .max_height(100.0)
                        .id_salt("window_neighborhood_summary_scroll")
                        .show(ui, |ui| {
                            ui.label(&data.summary);
                        });
                    ui.add_space(5.0);

                    // Themes
                    if !data.themes.is_empty() {
                        ui.label(egui::RichText::new("Themes").strong().color(Color32::from_rgb(100, 149, 237)));
                        ui.label(&data.themes);
                    }
                } else {
                    ui.colored_label(
                        egui::Color32::GRAY,
                        "Ctrl+Click a node to summarize\nit and its neighbors.",
                    );
                }

                ui.add_space(10.0);
                ui.separator();

                // Clear button - only clears neighborhood window state
                if ui.button("Clear & Close").clicked() {
                    self.neighborhood_summary_data = None;
                    self.neighborhood_summary_error = None;
                    self.neighborhood_summary_center_node = None;
                    self.neighborhood_summary_count = 0;
                    self.neighborhood_window_open = false;
                    self.neighborhood_window_dragged = false;
                }
            });

        self.neighborhood_window_open = open;

        // Detect if window was dragged from default position
        if let Some(inner) = window_response {
            let pos = inner.response.rect.left_top();
            let dist = (pos - default_pos).length();
            if dist > 10.0 {
                self.neighborhood_window_dragged = true;
            }
        }
    }

    /// Render the beads panel (issues/tasks)
    fn render_beads_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Beads");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new("B to toggle")
                        .small()
                        .color(theme::text::MUTED)
                );
            });
        });
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            // Placeholder content - beads data integration would go here
            ui.label(
                egui::RichText::new("Issue tracking panel")
                    .color(theme::text::SECONDARY)
            );
            ui.add_space(16.0);

            // Sample structure showing what the panel would contain
            ui.label(egui::RichText::new("Ready").strong());
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("No ready issues")
                    .color(theme::text::MUTED)
                    .italics()
            );

            ui.add_space(16.0);
            ui.label(egui::RichText::new("In Progress").strong());
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("No issues in progress")
                    .color(theme::text::MUTED)
                    .italics()
            );

            ui.add_space(16.0);
            ui.label(egui::RichText::new("Blocked").strong());
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("No blocked issues")
                    .color(theme::text::MUTED)
                    .italics()
            );
        });
    }

    /// Render the mail panel (inbox/outbox)
    fn render_mail_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Mail");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new("M to toggle")
                        .small()
                        .color(theme::text::MUTED)
                );
            });
        });
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        // Tab selection for Inbox/Outbox (placeholder - actual tab state would go here)
        ui.horizontal(|ui| {
            let _ = ui.selectable_label(true, "Inbox");
            let _ = ui.selectable_label(false, "Outbox");
        });
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            // Placeholder content - mail data integration would go here
            ui.label(
                egui::RichText::new("Mail panel")
                    .color(theme::text::SECONDARY)
            );
            ui.add_space(16.0);

            ui.label(
                egui::RichText::new("No messages")
                    .color(theme::text::MUTED)
                    .italics()
            );
        });
    }

    fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        // Tab bar at top
        let prev_tab = self.sidebar_tab;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.sidebar_tab, SidebarTab::Data, "Data");
            ui.selectable_value(&mut self.sidebar_tab, SidebarTab::Nodes, "Nodes");
            ui.selectable_value(&mut self.sidebar_tab, SidebarTab::Edges, "Edges");
            ui.selectable_value(&mut self.sidebar_tab, SidebarTab::Filters, "Filters");
        });
        if self.sidebar_tab != prev_tab {
            self.mark_settings_dirty();
        }
        ui.separator();

        // Tab content
        match self.sidebar_tab {
            SidebarTab::Data => self.render_sidebar_data(ui),
            SidebarTab::Nodes => self.render_sidebar_nodes(ui),
            SidebarTab::Edges => self.render_sidebar_edges(ui),
            SidebarTab::Filters => self.render_sidebar_filters(ui),
        }
    }

    fn render_sidebar_data(&mut self, ui: &mut egui::Ui) {
        // Database status
        ui.horizontal(|ui| {
            if self.db_connected {
                ui.colored_label(theme::state::SUCCESS, "● DB Connected");
            } else {
                ui.colored_label(theme::state::ERROR, "● DB Disconnected");
                if ui.button("Retry").clicked() {
                    self.reconnect_db();
                    if self.db_connected {
                        self.load_graph();
                    }
                }
            }
        });

        if let Some(ref err) = self.db_error {
            ui.colored_label(theme::state::ERROR, format!("Error: {}", err));
        }

        ui.add_space(10.0);

        // Data Selection section
        egui::CollapsingHeader::new("Data Selection")
            .default_open(true)
            .show(ui, |ui| {
                ui.label(format!("Range: {}", format_hours_label(self.slider_hours)));
                ui.add(
                    egui::Slider::new(&mut self.slider_hours, 1.0..=2160.0)
                        .logarithmic(true)
                        .clamping(egui::SliderClamping::Always)
                        .show_value(false),
                );
                let changed = (self.slider_hours - self.time_range_hours).abs() > 0.5;
                ui.add_enabled_ui(changed, |ui| {
                    if ui.button("Load").clicked() {
                        self.time_range_hours = self.slider_hours;
                        self.load_graph();
                        self.mark_settings_dirty();
                    }
                });

                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    if ui.button("⟳ Reload").clicked() {
                        self.load_graph();
                    }
                    if ui.button("↺ Reset All").clicked() {
                        // Reset all UI state to defaults
                        self.node_size = 15.0;
                        self.show_arrows = true;
                        self.graph.physics_enabled = true;
                        self.timeline_enabled = true;
                        // Reset sizing to Balanced preset
                        self.sizing_preset = SizingPreset::Balanced;
                        let (w_imp, w_tok, w_time) = SizingPreset::Balanced.weights();
                        self.w_importance = w_imp;
                        self.w_tokens = w_tok;
                        self.w_time = w_time;
                        self.layout.repulsion = 10000.0;
                        self.layout.attraction = 0.1;
                        self.layout.centering = 0.0001;
                        self.layout.size_physics_weight = 0.0;
                        self.layout.directed_stiffness = 1.0;
                        self.layout.recency_centering = 0.0;
                        self.layout.momentum = 0.0;
                        self.pan_offset = Vec2::ZERO;
                        self.zoom = 1.0;
                        self.load_graph();
                    }
                });

                // Re-ingest sessions from ~/.claude/
                if self.ingest_loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Ingesting sessions...");
                    });
                } else {
                    if ui.button("↻ Re-ingest Sessions").on_hover_text(
                        "Import new sessions from ~/.claude/ into the database"
                    ).clicked() {
                        self.start_ingest();
                    }
                }
                if let Some(ref result) = self.ingest_result {
                    ui.label(
                        egui::RichText::new(format!(
                            "Imported {} sessions, {} msgs",
                            result.sessions, result.messages
                        ))
                        .size(11.0)
                        .color(theme::state::SUCCESS),
                    );
                }

                // Last synced timestamp
                if let Some(last_synced) = self.last_synced {
                    let elapsed = last_synced.elapsed();
                    let elapsed_str = if elapsed.as_secs() < 60 {
                        format!("{}s ago", elapsed.as_secs())
                    } else if elapsed.as_secs() < 3600 {
                        format!("{}m ago", elapsed.as_secs() / 60)
                    } else {
                        format!("{}h ago", elapsed.as_secs() / 3600)
                    };
                    ui.label(format!("Last synced: {}", elapsed_str));
                }

                // Auto-refresh toggle
                ui.add_space(5.0);
                let mut auto_refresh = self.settings.auto_refresh_enabled;
                if ui.checkbox(&mut auto_refresh, "Auto-refresh").changed() {
                    self.settings.auto_refresh_enabled = auto_refresh;
                    self.mark_settings_dirty();
                }
                if self.settings.auto_refresh_enabled {
                    ui.horizontal(|ui| {
                        ui.label("Interval:");
                        let mut interval = self.settings.auto_refresh_interval_secs;
                        if ui.add(egui::DragValue::new(&mut interval)
                            .range(1.0..=60.0)
                            .suffix("s")
                            .speed(0.5)
                        ).changed() {
                            self.settings.auto_refresh_interval_secs = interval;
                            self.mark_settings_dirty();
                        }
                    });
                }
            });

        // Presets section
        egui::CollapsingHeader::new("Presets")
            .default_open(false)
            .show(ui, |ui| {
                // Dropdown to select a preset
                let preset_names: Vec<String> = self.settings.presets.iter().map(|p| p.name.clone()).collect();
                let selected_label = self.selected_preset_index
                    .and_then(|i| preset_names.get(i).cloned())
                    .unwrap_or_else(|| "Select preset...".to_string());

                egui::ComboBox::from_id_salt("preset_selector")
                    .selected_text(&selected_label)
                    .show_ui(ui, |ui| {
                        for (i, name) in preset_names.iter().enumerate() {
                            if ui.selectable_value(&mut self.selected_preset_index, Some(i), name).changed() {
                                // Apply the preset immediately on selection
                                if let Some(preset) = self.settings.presets.get(i).cloned() {
                                    preset.apply_to(&mut self.settings, &mut self.graph);
                                    self.sync_ui_from_settings();
                                    self.mark_settings_dirty();
                                }
                            }
                        }
                    });

                ui.add_space(5.0);

                // Save current settings as new preset
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.preset_name_input)
                        .hint_text("Preset name")
                        .desired_width(120.0));

                    if ui.button("Save").clicked() && !self.preset_name_input.trim().is_empty() {
                        let name = self.preset_name_input.trim().to_string();

                        // Check if preset with this name exists
                        if let Some(idx) = self.settings.presets.iter().position(|p| p.name == name) {
                            // Update existing
                            self.settings.presets[idx] = Preset::from_settings(name, &self.settings, &self.graph);
                            self.selected_preset_index = Some(idx);
                        } else {
                            // Add new
                            let preset = Preset::from_settings(name, &self.settings, &self.graph);
                            self.settings.presets.push(preset);
                            self.selected_preset_index = Some(self.settings.presets.len() - 1);
                        }
                        self.preset_name_input.clear();
                        self.mark_settings_dirty();
                    }
                });

                // Delete selected preset
                if self.selected_preset_index.is_some() {
                    ui.add_space(5.0);
                    if ui.button("Delete selected").clicked() {
                        if let Some(idx) = self.selected_preset_index {
                            if idx < self.settings.presets.len() {
                                self.settings.presets.remove(idx);
                                self.selected_preset_index = None;
                                self.mark_settings_dirty();
                            }
                        }
                    }
                }

                if self.settings.presets.is_empty() {
                    ui.add_space(5.0);
                    ui.label("No saved presets yet");
                }
            });

        ui.add_space(5.0);
        ui.separator();

        // View / Zoom controls
        ui.label("View");
        ui.horizontal(|ui| {
            if ui.button("Reset View").clicked() {
                self.pan_offset = Vec2::ZERO;
                self.zoom = 1.0;
            }
            ui.label(format!("Zoom: {:.0}%", self.zoom * 100.0));
        });

        ui.add_space(5.0);

        // Token Histogram toggle
        if ui.checkbox(&mut self.histogram_panel_enabled, "Token Histogram Panel")
            .on_hover_text("Show token usage histogram in a split pane")
            .changed()
        { self.mark_settings_dirty(); }

        ui.add_space(5.0);
        ui.separator();

        // Embedding stats and generation
        let stats_info = self.embedding_stats.as_ref().map(|s| (s.embedded, s.total, s.unembedded));
        let gen_loading = self.embedding_gen_loading;

        if let Some((embedded, total, unembedded)) = stats_info {
            ui.label(format!("{} / {} embedded", embedded, total));

            if unembedded > 0 {
                let gen_text = if gen_loading { "Generating..." } else { "Generate" };
                ui.horizontal(|ui| {
                    if gen_loading {
                        ui.spinner();
                    }
                    if ui.add_enabled(!gen_loading, egui::Button::new(gen_text)).clicked() {
                        self.trigger_embedding_generation();
                    }
                    if ui.add_enabled(!gen_loading, egui::Button::new("Visible only")).clicked() {
                        self.trigger_embedding_generation_visible();
                    }
                    ui.label(format!("{} remaining", unembedded));
                });
            }
        } else {
            if ui.button("Load stats").clicked() {
                self.load_embedding_stats();
            }
        }

        ui.add_space(5.0);
        ui.separator();

        // Info section
        ui.label("Info");
        let visible_count = self.effective_visible_count;
        let total_count = self.graph.data.nodes.len();
        if self.any_filter_active() && visible_count < total_count {
            ui.label(format!("Nodes: {} / {}", visible_count, total_count));
        } else {
            ui.label(format!("Nodes: {}", total_count));
        }
        ui.label(format!("Edges: {}", self.graph.data.edges.len()));
        ui.label(format!("FPS: {:.1}", self.fps));

        let user_count = self.graph.data.nodes.iter().filter(|n| n.role == crate::graph::types::Role::User).count();
        let assistant_count = self.graph.data.nodes.iter().filter(|n| n.role == crate::graph::types::Role::Assistant).count();
        ui.label(format!("You: {} | Claude: {}", user_count, assistant_count));
    }

    fn render_sidebar_nodes(&mut self, ui: &mut egui::Ui) {
        // Display section
        egui::CollapsingHeader::new("Display")
            .default_open(true)
            .show(ui, |ui| {
                if ui.add(egui::Slider::new(&mut self.node_size, 5.0..=50.0).text("Node size")).changed() {
                    self.mark_settings_dirty();
                }

                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    ui.label("Color by:");
                    if ui.selectable_label(self.graph.color_mode == ColorMode::Project, "Project")
                        .on_hover_text("All sessions in same project share the same color")
                        .clicked()
                    {
                        self.graph.color_mode = ColorMode::Project;
                        self.mark_settings_dirty();
                    }
                    if ui.selectable_label(self.graph.color_mode == ColorMode::Hybrid, "Hybrid")
                        .on_hover_text("Project hue + session shade (older=lighter, newer=darker)")
                        .clicked()
                    {
                        self.graph.color_mode = ColorMode::Hybrid;
                        self.mark_settings_dirty();
                    }
                    if ui.selectable_label(self.graph.color_mode == ColorMode::Session, "Session")
                        .on_hover_text("Each session gets its own unique color")
                        .clicked()
                    {
                        self.graph.color_mode = ColorMode::Session;
                        self.mark_settings_dirty();
                    }
                    ui.separator();
                    if ui.button("🎲").on_hover_text("Randomize hues").clicked() {
                        self.graph.randomize_hue_offset();
                    }
                });

                ui.add_space(5.0);
                ui.checkbox(&mut self.debug_tooltip, "Debug tooltip")
                    .on_hover_text("Show node classification and rendering debug info in tooltip");
            });

        // Node Sizing section
        egui::CollapsingHeader::new("Node Sizing")
            .default_open(true)
            .show(ui, |ui| {
                // Preset dropdown
                let current_label = self.sizing_preset.label();
                egui::ComboBox::from_id_salt("sizing_preset")
                    .selected_text(current_label)
                    .show_ui(ui, |ui| {
                        for preset in SizingPreset::all() {
                            if ui.selectable_label(
                                self.sizing_preset == *preset,
                                preset.label()
                            ).clicked() {
                                self.sizing_preset = *preset;
                                let (w_imp, w_tok, w_time) = preset.weights();
                                self.w_importance = w_imp;
                                self.w_tokens = w_tok;
                                self.w_time = w_time;
                                self.mark_settings_dirty();
                            }
                        }
                        // Show Custom as non-selectable label if currently custom
                        if self.sizing_preset == SizingPreset::Custom {
                            let _ = ui.selectable_label(true, "Custom");
                        }
                    });

                ui.add_space(5.0);

                // Weight sliders (log scale for wide range)
                if ui.add(egui::Slider::new(&mut self.w_importance, 0.01..=50.0)
                    .logarithmic(true)
                    .text("Importance")
                    .fixed_decimals(2)).changed() {
                    self.sizing_preset = SizingPreset::Custom;
                    self.mark_settings_dirty();
                }

                if ui.add(egui::Slider::new(&mut self.w_tokens, 0.01..=50.0)
                    .logarithmic(true)
                    .text("Tokens")
                    .fixed_decimals(2)).changed() {
                    self.sizing_preset = SizingPreset::Custom;
                    self.mark_settings_dirty();
                }

                if ui.add(egui::Slider::new(&mut self.w_time, 0.01..=50.0)
                    .logarithmic(true)
                    .text("Recency")
                    .fixed_decimals(2)).changed() {
                    self.sizing_preset = SizingPreset::Custom;
                    self.mark_settings_dirty();
                }

                ui.add_space(10.0);
                ui.separator();

                // Max node size slider
                if ui.add(egui::Slider::new(&mut self.max_node_multiplier, 0.1..=100.0)
                    .logarithmic(true)
                    .text("Max size")
                    .fixed_decimals(1)).changed() {
                    self.mark_settings_dirty();
                }

                // Help text
                ui.add_space(5.0);
                ui.label(egui::RichText::new("Largest node will be this multiple of base size").weak().small());
            });

        // Size → Physics weight
        egui::CollapsingHeader::new("Size → Physics")
            .default_open(false)
            .show(ui, |ui| {
                if ui.add(egui::Slider::new(&mut self.layout.size_physics_weight, 0.0..=5.0)
                    .text("Weight")
                    .fixed_decimals(2)).changed() {
                    self.mark_settings_dirty();
                }
                ui.label(egui::RichText::new("Higher = small nodes become less significant in physics").small().weak());
            });

        ui.add_space(5.0);
        ui.separator();

        // Legend
        if self.graph.color_mode != ColorMode::Session {
            ui.label(if self.graph.color_mode == ColorMode::Hybrid { "Projects (Hybrid)" } else { "Projects" });
            // Show top projects by color
            let mut projects: Vec<_> = self.graph.project_colors.iter().collect();
            projects.sort_by(|a, b| a.0.cmp(b.0));
            for (project, &hue) in projects.iter().take(8) {
                ui.horizontal(|ui| {
                    let color = crate::graph::types::hsl_to_rgb(hue, 0.7, 0.55);
                    ui.colored_label(color, "●");
                    let label = if project.len() > 15 {
                        format!("{}…", &project[..14])
                    } else {
                        project.to_string()
                    };
                    ui.label(label);
                });
            }
            if projects.len() > 8 {
                ui.label(format!("  +{} more", projects.len() - 8));
            }
        } else {
            ui.label("Legend");
            ui.horizontal(|ui| {
                ui.colored_label(Color32::WHITE, "●");
                ui.label("You");
            });
            ui.horizontal(|ui| {
                ui.colored_label(Color32::from_rgb(255, 149, 0), "●");
                ui.label("Claude");
            });
        }
    }

    fn render_sidebar_edges(&mut self, ui: &mut egui::Ui) {
        // Show arrows toggle
        if ui.checkbox(&mut self.show_arrows, "Show arrows").changed() {
            self.mark_settings_dirty();
        }

        ui.add_space(5.0);

        // --- Compact edge row helper ---
        // Each row: [toggle] Label    status_text [gear]

        // Physics row
        {
            let physics_visible = self.compute_physics_visible_nodes();
            let settled = self.layout.is_settled(&self.graph, physics_visible.as_ref());
            let status = if !self.graph.physics_enabled {
                "off".to_string()
            } else if settled {
                "settled".to_string()
            } else {
                "running".to_string()
            };

            ui.horizontal(|ui| {
                if ui.checkbox(&mut self.graph.physics_enabled, "Physics").changed() {
                    self.mark_settings_dirty();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let is_open = self.edge_popup_open == Some(EdgePopup::Physics);
                    let gear = if is_open {
                        egui::RichText::new("\u{2699}").strong()
                    } else {
                        egui::RichText::new("\u{2699}")
                    };
                    if ui.add(egui::Button::new(gear).frame(false)).clicked() {
                        self.edge_popup_open = if is_open { None } else { Some(EdgePopup::Physics) };
                    }
                    ui.label(egui::RichText::new(status).weak().small());
                });
            });
        }

        // Layout Shaping row
        {
            let status = if !self.layout_shaping_enabled {
                "off".to_string()
            } else {
                format!("stiffness: {:.1}", self.layout.directed_stiffness)
            };

            ui.horizontal(|ui| {
                let was_enabled = self.layout_shaping_enabled;
                if ui.checkbox(&mut self.layout_shaping_enabled, "Layout Shaping").changed() {
                    if !self.layout_shaping_enabled {
                        self.layout.directed_stiffness = 1.0;
                        self.layout.recency_centering = 0.0;
                    }
                    self.mark_settings_dirty();
                }
                // Restore if just reading (checkbox already handles mutation)
                let _ = was_enabled;

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let is_open = self.edge_popup_open == Some(EdgePopup::LayoutShaping);
                    let gear = if is_open {
                        egui::RichText::new("\u{2699}").strong()
                    } else {
                        egui::RichText::new("\u{2699}")
                    };
                    if ui.add(egui::Button::new(gear).frame(false)).clicked() {
                        self.edge_popup_open = if is_open { None } else { Some(EdgePopup::LayoutShaping) };
                    }
                    ui.label(egui::RichText::new(status).weak().small());
                });
            });
        }

        // Temporal Clustering row
        {
            let temporal_count = self.graph.data.edges.iter().filter(|e| e.is_temporal).count();
            let status = if !self.graph.temporal_attraction_enabled {
                "off".to_string()
            } else {
                format!("{} edges", temporal_count)
            };

            ui.horizontal(|ui| {
                let temporal_enabled = self.graph.temporal_attraction_enabled;
                let mut new_temporal_enabled = temporal_enabled;
                if ui.checkbox(&mut new_temporal_enabled, "Temporal Clustering").changed() {
                    self.graph.temporal_attraction_enabled = new_temporal_enabled;
                    self.temporal_edges_dirty = true;
                    self.mark_settings_dirty();
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let is_open = self.edge_popup_open == Some(EdgePopup::TemporalClustering);
                    let gear = if is_open {
                        egui::RichText::new("\u{2699}").strong()
                    } else {
                        egui::RichText::new("\u{2699}")
                    };
                    if ui.add(egui::Button::new(gear).frame(false)).clicked() {
                        self.edge_popup_open = if is_open { None } else { Some(EdgePopup::TemporalClustering) };
                    }
                    ui.label(egui::RichText::new(status).weak().small());
                });
            });
        }

        // Score-Proximity section (inline search + query management)
        ui.add_space(3.0);
        ui.separator();
        {
            let status = if !self.graph.score_proximity_enabled {
                "off".to_string()
            } else {
                let count = self.total_proximity_edge_count();
                format!("{} edges", count)
            };

            // Header row: toggle + label + status + gear
            ui.horizontal(|ui| {
                let was_enabled = self.graph.score_proximity_enabled;
                ui.checkbox(&mut self.graph.score_proximity_enabled, "Score-Proximity");
                if self.graph.score_proximity_enabled != was_enabled && !self.graph.score_proximity_enabled {
                    self.clear_proximity();
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let is_open = self.edge_popup_open == Some(EdgePopup::ScoreProximity);
                    let gear = if is_open {
                        egui::RichText::new("\u{2699}").strong()
                    } else {
                        egui::RichText::new("\u{2699}")
                    };
                    if ui.add(egui::Button::new(gear).frame(false)).clicked() {
                        self.edge_popup_open = if is_open { None } else { Some(EdgePopup::ScoreProximity) };
                    }
                    ui.label(egui::RichText::new(status).weak().small());
                });
            });

            // Inline controls: search, tags, queries (always visible when enabled)
            if self.graph.score_proximity_enabled {
                // Search input + Add button
                let mut add_query: Option<String> = None;
                ui.horizontal(|ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.proximity_input)
                            .hint_text("Search concept...")
                            .desired_width(130.0)
                    );

                    let can_add = !self.proximity_input.trim().is_empty()
                        && !self.proximity_queries.iter().any(|q| q.query == self.proximity_input.trim());

                    if ui.add_enabled(can_add, egui::Button::new("Search")).clicked()
                        || (response.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            && can_add)
                    {
                        let term = self.proximity_input.trim().to_string();
                        add_query = Some(term.clone());
                        if !self.settings.proximity_quick_tags.iter().any(|t| t == &term) {
                            self.settings.proximity_quick_tags.push(term);
                            self.mark_settings_dirty();
                        }
                        self.proximity_input.clear();
                    }
                });

                // Quick preset buttons
                let mut preset_add: Option<String> = None;
                let mut preset_remove: Option<usize> = None;
                let mut tag_remove: Option<usize> = None;
                let tags = self.settings.proximity_quick_tags.clone();
                ui.horizontal_wrapped(|ui| {
                    for (tag_idx, tag) in tags.iter().enumerate() {
                        let existing_idx = self.proximity_queries.iter().position(|q| &q.query == tag);
                        let is_active = existing_idx.is_some();
                        let button = egui::Button::new(
                            if is_active {
                                egui::RichText::new(tag.as_str()).underline()
                            } else {
                                egui::RichText::new(tag.as_str())
                            }
                        );
                        let response = ui.add(button);
                        if response.clicked() {
                            if let Some(idx) = existing_idx {
                                preset_remove = Some(idx);
                            } else {
                                preset_add = Some(tag.clone());
                            }
                        }
                        response.context_menu(|ui| {
                            if ui.button("Remove tag").clicked() {
                                tag_remove = Some(tag_idx);
                                ui.close_menu();
                            }
                        });
                    }
                });
                if let Some(idx) = tag_remove {
                    self.settings.proximity_quick_tags.remove(idx);
                    self.mark_settings_dirty();
                }

                // Apply deferred add/remove
                if let Some(text) = add_query {
                    self.add_proximity_query(text);
                }
                if let Some(text) = preset_add {
                    self.add_proximity_query(text);
                }
                if let Some(idx) = preset_remove {
                    self.remove_proximity_query(idx);
                }

                // Loading indicator
                if self.any_proximity_loading() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading...");
                    });
                }

                // Active queries as colored chip rows
                if !self.proximity_queries.is_empty() {
                    let mut remove_idx: Option<usize> = None;
                    for (i, q) in self.proximity_queries.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(8.0, 8.0),
                                egui::Sense::hover(),
                            );
                            ui.painter().circle_filled(rect.center(), 4.0, q.color);

                            let label = if q.loading {
                                format!("\"{}\" ...", q.query)
                            } else {
                                format!("\"{}\" ({} edges)", q.query, q.edge_count)
                            };
                            ui.label(egui::RichText::new(label).small());

                            if ui.small_button("x").clicked() {
                                remove_idx = Some(i);
                            }
                        });
                    }
                    if let Some(idx) = remove_idx {
                        self.remove_proximity_query(idx);
                    }
                }
            }
        }
    }

    // --- Edge popup body methods (rendered in floating windows) ---

    fn render_physics_popup(&mut self, ui: &mut egui::Ui) {
        if ui.add(egui::Slider::new(&mut self.layout.repulsion, 10.0..=100000.0).logarithmic(true).text("Repulsion")).changed() {
            self.mark_settings_dirty();
        }
        if ui.add(egui::Slider::new(&mut self.layout.attraction, 0.0001..=10.0).logarithmic(true).text("Attraction")).changed() {
            self.mark_settings_dirty();
        }
        if ui.add(egui::Slider::new(&mut self.layout.centering, 0.00001..=0.1).logarithmic(true).text("Centering")).changed() {
            self.mark_settings_dirty();
        }
        if ui.add(egui::Slider::new(&mut self.layout.momentum, 0.0..=0.95).fixed_decimals(2).text("Momentum")).changed() {
            self.mark_settings_dirty();
        }
    }

    fn render_layout_shaping_popup(&mut self, ui: &mut egui::Ui) {
        if self.layout_shaping_enabled {
            if ui.add(egui::Slider::new(&mut self.layout.directed_stiffness, 0.1..=20.0)
                .logarithmic(true)
                .text("Edge Stiffness")
                .fixed_decimals(2)).changed() {
                self.mark_settings_dirty();
            }
            ui.label(egui::RichText::new("Higher = tighter session chains").small().weak());

            ui.add_space(5.0);

            if ui.add(egui::Slider::new(&mut self.layout.recency_centering, 0.0..=50.0)
                .text("Recency\u{2192}Center")
                .fixed_decimals(1)).changed() {
                self.mark_settings_dirty();
            }
            ui.label(egui::RichText::new("Higher = newer nodes pulled to center").small().weak());
        } else {
            ui.label(egui::RichText::new("Enable Layout Shaping to configure.").weak());
        }
    }

    fn render_temporal_popup(&mut self, ui: &mut egui::Ui) {
        if !self.graph.temporal_attraction_enabled {
            ui.label(egui::RichText::new("Enable Temporal Clustering to configure.").weak());
            return;
        }

        if ui.add(egui::Slider::new(&mut self.layout.temporal_strength, 0.001..=2.0)
            .logarithmic(true)
            .text("Strength")).changed() {
            self.mark_settings_dirty();
        }

        // Temporal window slider (in minutes for UX, stored as seconds)
        let mut window_mins = (self.graph.temporal_window_secs / 60.0) as f32;
        let prev_window_mins = window_mins;
        ui.add(egui::Slider::new(&mut window_mins, 1.0..=60.0)
            .text("Window (min)")
            .fixed_decimals(0));
        if (window_mins - prev_window_mins).abs() > 0.1 {
            self.graph.temporal_window_secs = window_mins as f64 * 60.0;
            self.temporal_edges_dirty = true;
            self.mark_settings_dirty();
        }

        // Temporal edge opacity slider
        if ui.add(egui::Slider::new(&mut self.temporal_edge_opacity, 0.0..=1.0)
            .text("Edge opacity")
            .fixed_decimals(2)).changed() {
            self.mark_settings_dirty();
        }

        // Max temporal edges dropdown
        let edge_limits = [
            (10_000, "10k"),
            (50_000, "50k"),
            (100_000, "100k"),
            (250_000, "250k"),
            (500_000, "500k"),
            (1_000_000, "1M"),
        ];
        let current_limit = self.graph.max_temporal_edges;
        let current_label = edge_limits.iter()
            .find(|(v, _)| *v == current_limit)
            .map(|(_, l)| *l)
            .unwrap_or("Custom");

        ui.horizontal(|ui| {
            ui.label("Max edges:");
            egui::ComboBox::from_id_salt("max_temporal_edges")
                .selected_text(current_label)
                .show_ui(ui, |ui| {
                    for (value, label) in edge_limits {
                        if ui.selectable_label(current_limit == value, label).clicked() {
                            self.graph.max_temporal_edges = value;
                            self.temporal_edges_dirty = true;
                            self.settings.max_temporal_edges = value;
                            self.mark_settings_dirty();
                        }
                    }
                });
        });

        // Show temporal edge count
        let temporal_count = self.graph.data.edges.iter().filter(|e| e.is_temporal).count();
        ui.label(format!("Temporal edges: {}", temporal_count));
    }

    fn render_proximity_popup(&mut self, ui: &mut egui::Ui) {
        if !self.graph.score_proximity_enabled {
            ui.label(egui::RichText::new("Enable Score-Proximity to configure.").weak());
            return;
        }

        // Heat map dropdown (which query colors the overlay)
        if self.proximity_queries.len() > 1 {
            let heat_label = match self.proximity_heat_map_index {
                None => "All (max)".to_string(),
                Some(idx) => self.proximity_queries.get(idx)
                    .map(|q| q.query.clone())
                    .unwrap_or_else(|| "???".to_string()),
            };
            ui.horizontal(|ui| {
                ui.label("Heat map:");
                egui::ComboBox::from_id_salt("proximity_heat_map")
                    .selected_text(heat_label)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(self.proximity_heat_map_index.is_none(), "All (max)").clicked() {
                            self.proximity_heat_map_index = None;
                        }
                        for (i, q) in self.proximity_queries.iter().enumerate() {
                            if ui.selectable_label(self.proximity_heat_map_index == Some(i), &q.query).clicked() {
                                self.proximity_heat_map_index = Some(i);
                            }
                        }
                    });
            });
        }

        // Strength slider (physics force multiplier)
        ui.add(egui::Slider::new(&mut self.layout.similarity_strength, 0.001..=2.0)
            .logarithmic(true)
            .text("Strength"));

        // Delta slider
        let prev_delta = self.graph.score_proximity_delta;
        ui.add(egui::Slider::new(&mut self.graph.score_proximity_delta, 0.01..=0.5)
            .text("Delta (window)")
            .fixed_decimals(2));
        let delta_changed = (self.graph.score_proximity_delta - prev_delta).abs() > 0.001;

        // Edge opacity slider
        ui.add(egui::Slider::new(&mut self.proximity_edge_opacity, 0.0..=1.0)
            .text("Edge opacity")
            .fixed_decimals(2));

        // Stiffness slider
        ui.add(egui::Slider::new(&mut self.proximity_stiffness, 0.1..=10.0)
            .logarithmic(true)
            .text("Stiffness"));

        // Max edges dropdown
        let edge_limits = [
            (10_000_usize, "10k"),
            (50_000, "50k"),
            (100_000, "100k"),
            (250_000, "250k"),
            (500_000, "500k"),
            (1_000_000, "1M"),
        ];
        let current_limit = self.graph.max_proximity_edges;
        let current_label = edge_limits.iter()
            .find(|(v, _)| *v == current_limit)
            .map(|(_, l)| *l)
            .unwrap_or("Custom");
        let prev_max = self.graph.max_proximity_edges;

        ui.horizontal(|ui| {
            ui.label("Max edges:");
            egui::ComboBox::from_id_salt("max_proximity_edges")
                .selected_text(current_label)
                .show_ui(ui, |ui| {
                    for (value, label) in edge_limits {
                        if ui.selectable_label(current_limit == value, label).clicked() {
                            self.graph.max_proximity_edges = value;
                        }
                    }
                });
        });
        let max_changed = self.graph.max_proximity_edges != prev_max;

        // Max neighbors per node dropdown
        let neighbor_limits: [(usize, &str); 8] = [
            (0, "Unlimited"),
            (1, "1"),
            (2, "2"),
            (3, "3"),
            (5, "5"),
            (8, "8"),
            (12, "12"),
            (20, "20"),
        ];
        let current_neighbors = self.graph.max_neighbors_per_node;
        let current_n_label = neighbor_limits.iter()
            .find(|(v, _)| *v == current_neighbors)
            .map(|(_, l)| *l)
            .unwrap_or("Custom");
        let prev_neighbors = self.graph.max_neighbors_per_node;

        ui.horizontal(|ui| {
            ui.label("Max neighbors:");
            egui::ComboBox::from_id_salt("max_neighbors_per_node")
                .selected_text(current_n_label)
                .show_ui(ui, |ui| {
                    for (value, label) in neighbor_limits {
                        if ui.selectable_label(current_neighbors == value, label).clicked() {
                            self.graph.max_neighbors_per_node = value;
                        }
                    }
                });
        });
        let max_n_changed = self.graph.max_neighbors_per_node != prev_neighbors;

        ui.separator();

        // Show total edge count and scored nodes
        let display_count = if self.graph.max_neighbors_per_node > 0 {
            self.proximity_edge_count_filtered
        } else {
            self.total_proximity_edge_count()
        };
        ui.label(format!("Total edges: {}", display_count));
        if self.any_proximity_active() {
            let all_scores: HashSet<&String> = self.proximity_queries.iter()
                .filter(|q| q.active)
                .flat_map(|q| q.scores.keys())
                .collect();
            let matched = self.graph.data.nodes.iter()
                .filter(|n| all_scores.contains(&n.id))
                .count();
            ui.label(format!("Scored nodes: {} / {}", matched, self.graph.data.nodes.len()));
        }

        // Rebuild All / Clear All buttons
        ui.horizontal(|ui| {
            if ui.add_enabled(!self.any_proximity_loading() && !self.proximity_queries.is_empty(),
                egui::Button::new("Rebuild All")).clicked()
            {
                self.refetch_all_proximity_queries();
            }
            if ui.button("Clear All").clicked() {
                self.clear_proximity();
            }
        });

        // Auto-refetch when delta, max_edges, or max_neighbors change
        if (delta_changed || max_changed || max_n_changed) && !self.any_proximity_loading() && !self.proximity_queries.is_empty() {
            self.refetch_all_proximity_queries();
        }
    }

    /// Render floating popup windows for edge settings (called from update() at ctx level)
    fn render_edge_popups(&mut self, ctx: &egui::Context) {
        let popup = match self.edge_popup_open {
            Some(p) => p,
            None => return,
        };

        let (title, popup_variant) = match popup {
            EdgePopup::Physics => ("Physics Settings", EdgePopup::Physics),
            EdgePopup::LayoutShaping => ("Layout Shaping", EdgePopup::LayoutShaping),
            EdgePopup::TemporalClustering => ("Temporal Clustering", EdgePopup::TemporalClustering),
            EdgePopup::ScoreProximity => ("Score-Proximity Edges", EdgePopup::ScoreProximity),
        };

        let mut open = true;

        // Default position: just right of sidebar
        let default_x = 240.0;
        let default_y = 100.0;

        let window = egui::Window::new(title)
            .open(&mut open)
            .default_pos([default_x, default_y])
            .resizable(false)
            .collapsible(false);

        // Score-Proximity gets a scroll area due to its length
        if matches!(popup_variant, EdgePopup::ScoreProximity) {
            window.default_size([320.0, 500.0])
                .resizable(true)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        self.render_proximity_popup(ui);
                    });
                });
        } else {
            window.auto_sized()
                .show(ctx, |ui| {
                    match popup_variant {
                        EdgePopup::Physics => self.render_physics_popup(ui),
                        EdgePopup::LayoutShaping => self.render_layout_shaping_popup(ui),
                        EdgePopup::TemporalClustering => self.render_temporal_popup(ui),
                        EdgePopup::ScoreProximity => unreachable!(),
                    }
                });
        }

        if !open {
            self.edge_popup_open = None;
        }
    }

    fn render_sidebar_filters(&mut self, ui: &mut egui::Ui) {
        // Timeline controls
        egui::CollapsingHeader::new("Timeline")
            .default_open(true)
            .show(ui, |ui| {
                if ui.checkbox(&mut self.timeline_enabled, "Timeline scrubber").changed() {
                    self.effective_visible_dirty = true;
                    self.mark_settings_dirty();
                }
                if self.timeline_enabled {
                    if ui.checkbox(&mut self.hover_scrubs_timeline, "Hover scrubs timeline")
                        .on_hover_text("Same-session hover scrubs instantly; click to jump to cross-session nodes")
                        .changed()
                    {
                        self.mark_settings_dirty();
                    }
                }
            });

        // Importance filter
        egui::CollapsingHeader::new("Importance")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let mut mode_changed = false;
                    for &mode in &[FilterMode::Off, FilterMode::Inactive, FilterMode::Filtered] {
                        if ui.selectable_label(self.importance_filter == mode, mode.label()).clicked() {
                            self.importance_filter = mode;
                            mode_changed = true;
                        }
                    }
                    if mode_changed {
                        self.recompute_bypass_edges();
                        self.semantic_filter_cache = None;
                        self.effective_visible_dirty = true;
                        self.mark_settings_dirty();
                    }
                });
                if self.importance_filter.is_active() {
                    if ui.add(egui::Slider::new(&mut self.importance_threshold, 0.0..=1.0)
                        .text("Min importance")
                        .fixed_decimals(2)).changed() {
                        self.effective_visible_dirty = true;
                        self.mark_settings_dirty();
                    }
                    // Show count
                    let visible = self.graph.data.nodes.iter()
                        .filter(|n| n.importance_score.map_or(true, |s| s >= self.importance_threshold))
                        .count();
                    ui.label(format!("Showing: {} / {} nodes", visible, self.graph.data.nodes.len()));
                }

                // Show importance scoring stats
                if let Some(ref stats) = self.importance_stats {
                    ui.add_space(5.0);
                    ui.label(format!("Scored: {} / {}", stats.scored_messages, stats.total_messages));
                }

                // Rescore button
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    let button_enabled = !self.rescore_loading && !self.graph.data.nodes.is_empty();
                    if ui.add_enabled(button_enabled, egui::Button::new("Rescore Visible")).clicked() {
                        self.start_rescore_visible();
                    }
                });

                // Show rescore progress or result
                if self.rescore_loading {
                    if let Some(ref progress) = self.rescore_progress {
                        // Show progress bar and text
                        let fraction = if progress.total > 0 {
                            (progress.current + 1) as f32 / progress.total as f32
                        } else {
                            0.0
                        };
                        ui.add(egui::ProgressBar::new(fraction)
                            .text(format!("{}/{}", progress.current + 1, progress.total)));
                        ui.label(format!("Session {}... ({} msgs)",
                            progress.session_id, progress.messages_so_far));
                    } else {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Starting...");
                        });
                    }
                } else if let Some(ref result) = self.rescore_result {
                    if result.errors.is_empty() {
                        ui.label(format!("Rescored {} msgs in {} sessions",
                            result.messages_rescored, result.sessions_processed));
                    } else {
                        ui.colored_label(Color32::YELLOW, format!(
                            "Rescored {} msgs, {} errors",
                            result.messages_rescored, result.errors.len()
                        ));
                    }
                }
            });

        // Project filter
        egui::CollapsingHeader::new("Project")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let mut mode_changed = false;
                    for &mode in &[FilterMode::Off, FilterMode::Inactive, FilterMode::Filtered] {
                        if ui.selectable_label(self.project_filter == mode, mode.label()).clicked() {
                            self.project_filter = mode;
                            mode_changed = true;
                        }
                    }
                    if mode_changed {
                        self.recompute_bypass_edges();
                        self.semantic_filter_cache = None;
                        self.effective_visible_dirty = true;
                        self.mark_settings_dirty();
                    }
                });
                if self.project_filter.is_active() && !self.available_projects.is_empty() {
                    if let Some(tree) = self.project_tree.clone() {
                        ui.indent("project_filter_tree", |ui| {
                            ui.horizontal(|ui| {
                                if ui.small_button("All").clicked() {
                                    self.selected_projects = self.available_projects.iter().cloned().collect();
                                    self.effective_visible_dirty = true;
                                }
                                if ui.small_button("None").clicked() {
                                    self.selected_projects.clear();
                                    self.effective_visible_dirty = true;
                                }
                            });
                            egui::ScrollArea::vertical()
                                .max_height(200.0)
                                .show(ui, |ui| {
                                    for child in &tree.children {
                                        self.render_project_tree_node(ui, child);
                                    }
                                });
                            let visible = self.graph.data.nodes.iter()
                                .filter(|n| self.selected_projects.contains(&n.project))
                                .count();
                            ui.label(format!("{} / {} nodes", visible, self.graph.data.nodes.len()));
                        });
                    }
                }
            });

        // Hide tool uses
        egui::CollapsingHeader::new("Tool Uses")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let mut mode_changed = false;
                    for &mode in &[FilterMode::Off, FilterMode::Inactive, FilterMode::Filtered] {
                        if ui.selectable_label(self.tool_use_filter == mode, mode.label()).clicked() {
                            self.tool_use_filter = mode;
                            mode_changed = true;
                        }
                    }
                    if mode_changed {
                        self.recompute_bypass_edges();
                        self.semantic_filter_cache = None;
                        self.effective_visible_dirty = true;
                        self.mark_settings_dirty();
                    }
                });
                if self.tool_use_filter.is_active() {
                    let tool_count = self.graph.data.nodes.iter().filter(|n| n.has_tool_usage).count();
                    let total = self.graph.data.nodes.len();
                    ui.label(format!("Hiding: {} / {} nodes", tool_count, total));
                }
            });

        // Semantic Filters section
        egui::CollapsingHeader::new("Semantic Filters")
            .default_open(false)
            .show(ui, |ui| {
                // Loading indicator with skeleton
                if self.semantic_filter_loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading filters...");
                    });
                    ui.add_space(4.0);
                    // Skeleton filter items
                    for _ in 0..2 {
                        ui.horizontal(|ui| {
                            theme::skeleton_rect(ui, 60.0, 18.0);
                            ui.add_space(4.0);
                            theme::skeleton_rect(ui, 80.0, 18.0);
                        });
                    }
                }

                // Categorization in progress indicator (only when detail panel is collapsed)
                if let Some(filter_id) = self.categorizing_filter_id {
                    if !self.expanded_filter_ids.contains(&filter_id) {
                        let filter_name = self.semantic_filters.iter()
                            .find(|f| f.id == filter_id)
                            .map(|f| f.name.clone())
                            .unwrap_or_default();
                        if let Some((scored, total)) = self.categorization_progress {
                            let fraction = if total > 0 { scored as f32 / total as f32 } else { 0.0 };
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(format!("Scoring '{}' ({}/{})", filter_name, scored, total));
                            });
                            ui.add(egui::ProgressBar::new(fraction).animate(true));
                        } else {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(format!("Scoring '{}'...", filter_name));
                            });
                        }
                    }
                }

                // List existing filters with five-state toggles
                if !self.semantic_filters.is_empty() {
                    let filters = self.semantic_filters.clone();
                    let total_nodes = self.graph.data.nodes.len() as i64;
                    for filter in &filters {
                        let is_expanded = self.expanded_filter_ids.contains(&filter.id);

                        ui.horizontal(|ui| {
                            // Get current mode for this filter
                            let current_mode = self.semantic_filter_modes
                                .get(&filter.id)
                                .copied()
                                .unwrap_or(SemanticFilterMode::Off);

                            let inactive = theme::filter::INACTIVE;
                            let active_neutral = theme::filter::ACTIVE;
                            let active_green = theme::filter::INCLUDE;
                            let active_blue = theme::filter::INCLUDE_PLUS1;
                            let active_purple = theme::filter::INCLUDE_PLUS2;
                            let active_red = theme::filter::EXCLUDE;

                            // Off button (O)
                            let off_color = if current_mode == SemanticFilterMode::Off { active_neutral } else { inactive };
                            if ui.add(egui::Button::new("O").fill(off_color).min_size(Vec2::new(20.0, 18.0)))
                                .on_hover_text("Off - filter not applied")
                                .clicked()
                            {
                                self.semantic_filter_modes.insert(filter.id, SemanticFilterMode::Off);
                                self.semantic_filter_cache = None;
                                self.effective_visible_dirty = true;
                            }

                            // Exclude button (-)
                            let exclude_color = if current_mode == SemanticFilterMode::Exclude { active_red } else { inactive };
                            if ui.add(egui::Button::new("-").fill(exclude_color).min_size(Vec2::new(20.0, 18.0)))
                                .on_hover_text("Exclude - hide matching nodes")
                                .clicked()
                            {
                                self.semantic_filter_modes.insert(filter.id, SemanticFilterMode::Exclude);
                                self.semantic_filter_cache = None;
                                self.effective_visible_dirty = true;
                            }

                            // Include button (+)
                            let include_color = if current_mode == SemanticFilterMode::Include { active_green } else { inactive };
                            if ui.add(egui::Button::new("+").fill(include_color).min_size(Vec2::new(20.0, 18.0)))
                                .on_hover_text("Include - only show matching nodes")
                                .clicked()
                            {
                                self.semantic_filter_modes.insert(filter.id, SemanticFilterMode::Include);
                                self.semantic_filter_cache = None;
                                self.effective_visible_dirty = true;
                            }

                            // Include +1 button (show matching + direct neighbors)
                            let plus1_color = if current_mode == SemanticFilterMode::IncludePlus1 { active_blue } else { inactive };
                            if ui.add(egui::Button::new("+1").fill(plus1_color).min_size(Vec2::new(24.0, 18.0)))
                                .on_hover_text("Include +1 - show matching nodes + their direct neighbors")
                                .clicked()
                            {
                                self.semantic_filter_modes.insert(filter.id, SemanticFilterMode::IncludePlus1);
                                self.semantic_filter_cache = None;
                                self.effective_visible_dirty = true;
                            }

                            // Include +2 button (show matching + neighbors up to depth 2)
                            let plus2_color = if current_mode == SemanticFilterMode::IncludePlus2 { active_purple } else { inactive };
                            if ui.add(egui::Button::new("+2").fill(plus2_color).min_size(Vec2::new(24.0, 18.0)))
                                .on_hover_text("Include +2 - show matching nodes + neighbors up to 2 hops")
                                .clicked()
                            {
                                self.semantic_filter_modes.insert(filter.id, SemanticFilterMode::IncludePlus2);
                                self.semantic_filter_cache = None;
                                self.effective_visible_dirty = true;
                            }

                            // Type badge for rule filters
                            if filter.is_rule() {
                                ui.label(egui::RichText::new("[R]").small().color(egui::Color32::from_rgb(100, 200, 100)));
                            }

                            // Clickable filter name to toggle detail panel
                            let label_text = format!("{} ({}/{})", filter.name, filter.matches, filter.total_scored);
                            let label_response = ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&label_text).underline()
                                ).sense(egui::Sense::click())
                            );
                            if label_response.clicked() {
                                if self.expanded_filter_ids.contains(&filter.id) {
                                    self.expanded_filter_ids.remove(&filter.id);
                                } else {
                                    self.expanded_filter_ids.insert(filter.id);
                                }
                            }
                            label_response.on_hover_text("Click to toggle details");

                            // Delete button
                            ui.add_enabled_ui(self.categorizing_filter_id.is_none(), |ui| {
                                if ui.small_button("X").on_hover_text("Delete filter").clicked() {
                                    self.delete_semantic_filter(filter.id);
                                }
                            });
                        });

                        // Expanded detail panel
                        if is_expanded {
                            let is_categorizing = self.categorizing_filter_id == Some(filter.id);

                            egui::Frame::none()
                                .fill(ui.visuals().faint_bg_color)
                                .rounding(4.0)
                                .inner_margin(egui::Margin::same(6.0))
                                .outer_margin(egui::Margin::symmetric(0.0, 2.0))
                                .show(ui, |ui| {
                                    ui.style_mut().spacing.item_spacing.y = 3.0;

                                    // Query text
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new("Query:").weak());
                                        ui.label(&filter.query_text);
                                    });

                                    // Unscored count
                                    let unscored = total_nodes - filter.total_scored;
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new("Unscored:").weak());
                                        if unscored > 0 {
                                            ui.label(egui::RichText::new(format!("{}", unscored)).color(egui::Color32::from_rgb(255, 180, 80)));
                                        } else {
                                            ui.label("0");
                                        }
                                    });

                                    // Mode counts
                                    let match_count = filter.matches;
                                    let exclude_count = total_nodes.saturating_sub(match_count);

                                    // Compute +1 and +2 neighbor expansions
                                    let matching_ids: HashSet<String> = self.graph.data.nodes.iter()
                                        .filter(|n| n.semantic_filter_matches.contains(&filter.id))
                                        .map(|n| n.id.clone())
                                        .collect();
                                    let adj = self.build_adjacency_list(true);
                                    let plus1_set = expand_to_neighbors(&matching_ids, 1, &adj);
                                    let plus2_set = expand_to_neighbors(&matching_ids, 2, &adj);

                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new("Visibility by mode:").weak());
                                    });
                                    ui.horizontal(|ui| {
                                        ui.add_space(8.0);
                                        ui.label(egui::RichText::new(format!("+ {}", match_count)).color(theme::filter::INCLUDE));
                                        ui.label(egui::RichText::new("|").weak());
                                        ui.label(egui::RichText::new(format!("+1 {}", plus1_set.len())).color(theme::filter::INCLUDE_PLUS1));
                                        ui.label(egui::RichText::new("|").weak());
                                        ui.label(egui::RichText::new(format!("+2 {}", plus2_set.len())).color(theme::filter::INCLUDE_PLUS2));
                                        ui.label(egui::RichText::new("|").weak());
                                        ui.label(egui::RichText::new(format!("- {}", exclude_count)).color(theme::filter::EXCLUDE));
                                    });

                                    ui.add_space(2.0);

                                    // Rescore button + progress (hidden for rule filters)
                                    if filter.is_rule() {
                                        ui.label(egui::RichText::new("Rule (auto-scored)").weak().italics());
                                    } else if is_categorizing {
                                        if let Some((scored, total)) = self.categorization_progress {
                                            let fraction = if total > 0 { scored as f32 / total as f32 } else { 0.0 };
                                            ui.horizontal(|ui| {
                                                ui.spinner();
                                                ui.label(format!("Scoring... ({}/{})", scored, total));
                                            });
                                            ui.add(egui::ProgressBar::new(fraction).animate(true));
                                        } else {
                                            ui.horizontal(|ui| {
                                                ui.spinner();
                                                ui.label("Scoring...");
                                            });
                                        }
                                    } else {
                                        ui.add_enabled_ui(self.categorizing_filter_id.is_none(), |ui| {
                                            if ui.button("Rescore").on_hover_text("Categorize all messages with this filter").clicked() {
                                                self.trigger_categorization(filter.id);
                                            }
                                        });
                                    }
                                });
                        }
                    }
                    ui.add_space(5.0);
                } else if !self.semantic_filter_loading {
                    ui.label("No filters defined");
                    ui.add_space(5.0);
                }

                // Add new filter input
                ui.horizontal(|ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.new_filter_input)
                            .hint_text("New filter...")
                            .desired_width(120.0)
                    );

                    let can_add = !self.new_filter_input.trim().is_empty()
                        && self.categorizing_filter_id.is_none();

                    if ui.add_enabled(can_add, egui::Button::new("+")).clicked()
                        || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && can_add)
                    {
                        self.create_semantic_filter();
                    }
                });

                // Preset rule filter buttons (tree layout)
                {
                    let existing_queries: std::collections::HashSet<String> = self.semantic_filters.iter()
                        .map(|f| f.query_text.clone())
                        .collect();

                    // Helper closure to render a preset button
                    let mut preset_btn = |ui: &mut egui::Ui, label: &str, query: &str, this: &mut Self| {
                        let already_exists = existing_queries.contains(query);
                        ui.add_enabled_ui(!already_exists, |ui| {
                            let btn = ui.small_button(label);
                            if already_exists {
                                btn.on_hover_text(format!("'{}' already exists", query));
                            } else if btn.on_hover_text(format!("Create rule: {}", query)).clicked() {
                                this.create_rule_filter(label, query);
                            }
                        });
                    };

                    // Top-level presets
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Presets:").weak().small());
                        preset_btn(ui, "User", "role:user", self);
                        preset_btn(ui, "Assistant", "role:assistant", self);
                        preset_btn(ui, "Has Tools", "has_tools", self);
                        preset_btn(ui, "Long", "long", self);
                        preset_btn(ui, "Short", "short", self);
                    });

                    // Collapsible tool presets tree
                    egui::CollapsingHeader::new(egui::RichText::new("By Tool").weak().small())
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.style_mut().spacing.item_spacing.y = 2.0;

                            let tool_groups: &[(&str, &[(&str, &str)])] = &[
                                ("Core", &[
                                    ("Bash", "tool:Bash"),
                                    ("Read", "tool:Read"),
                                    ("Edit", "tool:Edit"),
                                    ("Write", "tool:Write"),
                                    ("Glob", "tool:Glob"),
                                    ("Grep", "tool:Grep"),
                                ]),
                                ("Web", &[
                                    ("WebSearch", "tool:WebSearch"),
                                    ("WebFetch", "tool:WebFetch"),
                                ]),
                                ("Agent", &[
                                    ("Task", "tool:Task"),
                                    ("TodoWrite", "tool:TodoWrite"),
                                    ("Skill", "tool:Skill"),
                                ]),
                                ("Planning", &[
                                    ("EnterPlan", "tool:EnterPlanMode"),
                                    ("ExitPlan", "tool:ExitPlanMode"),
                                    ("AskUser", "tool:AskUserQuestion"),
                                ]),
                            ];

                            for (group_name, tools) in tool_groups {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(egui::RichText::new(format!("{}:", group_name)).weak().small());
                                    for (label, query) in *tools {
                                        preset_btn(ui, label, query, self);
                                    }
                                });
                            }
                        });
                }

                // Show active filter count and effect
                let active_filters: Vec<_> = self.semantic_filter_modes.iter()
                    .filter(|(_, mode)| **mode != SemanticFilterMode::Off)
                    .collect();
                if !active_filters.is_empty() {
                    ui.add_space(5.0);
                    let include_count = active_filters.iter()
                        .filter(|(_, mode)| matches!(mode, SemanticFilterMode::Include | SemanticFilterMode::IncludePlus1 | SemanticFilterMode::IncludePlus2))
                        .count();
                    let exclude_count = active_filters.iter()
                        .filter(|(_, mode)| **mode == SemanticFilterMode::Exclude)
                        .count();
                    ui.label(format!(
                        "Active: {} include, {} exclude",
                        include_count,
                        exclude_count
                    ));
                }
            });

        ui.add_space(10.0);
        ui.separator();

        // Mail Network Graph section
        egui::CollapsingHeader::new("Mail Network")
            .default_open(false)
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Agent Communication").size(11.0).color(Color32::GRAY));

                // Load button
                if self.mail_network_loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading...");
                    });
                } else if ui.button("Load Mail Network").clicked() {
                    self.load_mail_network();
                }

                // Error message
                if let Some(ref err) = self.mail_network_error {
                    ui.colored_label(Color32::RED, format!("Error: {}", err));
                }

                // Render the mail network graph
                if let Some(ref mut state) = self.mail_network_state {
                    ui.add_space(5.0);
                    let size = Vec2::new(ui.available_width().min(250.0), 200.0);
                    render_mail_network(ui, state, size);
                }
            });

        ui.add_space(10.0);
        ui.separator();

        // Node at Scrubber Position
        ui.label("Node at Scrubber");
        if let Some(closest_node) = self.find_node_at_scrubber() {
            ui.horizontal(|ui| {
                let role_color = self.graph.node_color(&closest_node);
                ui.colored_label(role_color, "●");
                ui.label(closest_node.role.label());
            });
            if let Some(ref ts) = closest_node.timestamp {
                // Format timestamp using the timeline's format_time function for consistency
                if let Some(epoch_secs) = closest_node.timestamp_secs() {
                    let formatted = self.graph.timeline.format_time(epoch_secs);
                    ui.label(format!("Time: {}", formatted));
                } else {
                    // Debug: parsing failed, show what we got
                    eprintln!("Failed to parse timestamp: {}", ts);
                    // Fallback: show just the time portion if parsing fails
                    let time_display = if let Some(t_idx) = ts.find('T') {
                        let time_part = &ts[t_idx + 1..];
                        if let Some(plus_idx) = time_part.find('+') {
                            &time_part[..plus_idx.min(8)]
                        } else {
                            &time_part[..8.min(time_part.len())]
                        }
                    } else {
                        ts.as_str()
                    };
                    ui.label(format!("Time: {}", time_display));
                }
            }
            ui.label(format!("Session: {}", closest_node.session_short));
            if !closest_node.project.is_empty() {
                ui.label(format!("Project: {}", closest_node.project));
            }

            // Content preview with word wrap
            ui.add_space(5.0);
            let preview = if closest_node.content_preview.chars().count() > 100 {
                let truncated: String = closest_node.content_preview.chars().take(100).collect();
                format!("{}...", truncated)
            } else {
                closest_node.content_preview.clone()
            };
            egui::ScrollArea::vertical()
                .max_height(80.0)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(preview).small());
                });
        } else {
            ui.label("No nodes loaded");
        }
    }

    /// Recursively render one node in the project tree with tri-state checkboxes.
    fn render_project_tree_node(&mut self, ui: &mut egui::Ui, node: &ProjectTreeNode) {
        let state = node.check_state(&self.selected_projects);
        let has_children = !node.children.is_empty();

        if has_children {
            // Interior node: collapsible with tri-state checkbox
            let is_expanded = self.project_tree_expanded.contains(&node.full_path);
            ui.horizontal(|ui| {
                // Expand/collapse arrow
                let arrow = if is_expanded { "\u{25BC}" } else { "\u{25B6}" };
                if ui.small_button(arrow).clicked() {
                    if is_expanded {
                        self.project_tree_expanded.remove(&node.full_path);
                    } else {
                        self.project_tree_expanded.insert(node.full_path.clone());
                    }
                }
                // Tri-state checkbox
                if let Some(select_all) = project_tree::tri_state_checkbox(ui, state) {
                    let leaves = node.leaf_paths();
                    if select_all {
                        for p in leaves { self.selected_projects.insert(p); }
                    } else {
                        for p in &leaves { self.selected_projects.remove(p); }
                    }
                    self.effective_visible_dirty = true;
                }
                ui.label(&node.name);
            });
            if is_expanded {
                ui.indent(&node.full_path, |ui| {
                    for child in &node.children {
                        self.render_project_tree_node(ui, child);
                    }
                });
            }
        } else {
            // Leaf node: simple checkbox
            ui.horizontal(|ui| {
                // Small indent to align with children of interior nodes
                ui.add_space(ui.spacing().indent);
                let mut selected = state == CheckState::Checked;
                if ui.checkbox(&mut selected, &node.name).changed() {
                    if selected {
                        self.selected_projects.insert(node.full_path.clone());
                    } else {
                        self.selected_projects.remove(&node.full_path);
                    }
                    self.effective_visible_dirty = true;
                }
            });
        }
    }

    /// Render the first-run / empty-database welcome screen.
    fn render_empty_state(&mut self, ui: &mut egui::Ui) {
        let available = ui.available_size();
        ui.allocate_new_ui(
            egui::UiBuilder::new().max_rect(egui::Rect::from_center_size(
                ui.max_rect().center(),
                egui::vec2(available.x.min(520.0), available.y),
            )),
            |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(available.y * 0.25);

                    ui.label(
                        egui::RichText::new("Welcome to Claude Activity Dashboard")
                            .size(22.0)
                            .color(theme::text::PRIMARY)
                            .strong(),
                    );
                    ui.add_space(12.0);

                    // Show DB error if present, otherwise show empty-data guidance
                    if let Some(ref err) = self.db_error {
                        ui.label(
                            egui::RichText::new(format!("Database error: {}", err))
                                .size(14.0)
                                .color(theme::accent::RED),
                        );
                        ui.add_space(8.0);
                        if ui.button("Retry connection").clicked() {
                            self.reconnect_db();
                            if self.db_connected {
                                self.load_graph();
                            }
                        }
                    } else {
                        ui.label(
                            egui::RichText::new(
                                "No conversation data found.\nImport your Claude Code history to get started.",
                            )
                            .size(14.0)
                            .color(theme::text::SECONDARY),
                        );
                    }

                    ui.add_space(20.0);

                    // Instructions
                    egui::Frame::none()
                        .fill(theme::bg::SURFACE)
                        .rounding(6.0)
                        .inner_margin(egui::Margin::same(16.0))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new("Getting started")
                                    .size(14.0)
                                    .color(theme::text::PRIMARY)
                                    .strong(),
                            );
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new(
                                    "Run the ingestion tool to import sessions:\n\n\
                                     dashboard-native ingest\n\n\
                                     Or import only recent history:\n\n\
                                     dashboard-native ingest --since 7d",
                                )
                                .size(13.0)
                                .color(theme::text::SECONDARY)
                                .family(egui::FontFamily::Monospace),
                            );
                        });

                    ui.add_space(16.0);

                    // Database path info
                    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| {
                        dirs::config_dir()
                            .unwrap_or_else(|| std::path::PathBuf::from("."))
                            .join("dashboard-native")
                            .join("dashboard.db")
                            .to_string_lossy()
                            .to_string()
                    });
                    ui.label(
                        egui::RichText::new(format!("Database: {}", db_path))
                            .size(11.0)
                            .color(theme::text::MUTED),
                    );

                    ui.add_space(16.0);

                    if ui.button("Refresh").clicked() {
                        if self.db_connected {
                            self.load_graph();
                        } else {
                            self.reconnect_db();
                            if self.db_connected {
                                self.load_graph();
                            }
                        }
                    }
                });
            },
        );
    }

    fn render_graph(&mut self, ui: &mut egui::Ui) {
        let (response, painter) = ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
        let rect = response.rect;
        let center = rect.center();

        // Gather all input deltas first (allows simultaneous pan+zoom on trackpad)
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
        let zoom_delta = ui.input(|i| i.zoom_delta());
        let hover_pos = response.hover_pos();

        // Handle click-drag pan (for mouse users)
        if response.dragged_by(egui::PointerButton::Primary) {
            self.pan_offset += response.drag_delta();
        }

        // Handle two-finger scroll pan (for trackpad users)
        // Apply before zoom so cursor-anchored zoom works correctly
        if scroll_delta != egui::Vec2::ZERO && response.hovered() {
            self.pan_offset += scroll_delta;
        }

        // Handle pinch-to-zoom and Ctrl+scroll (cursor-anchored)
        if let Some(cursor_pos) = hover_pos {
            if zoom_delta != 1.0 {
                let new_zoom = (self.zoom * zoom_delta).clamp(0.005, 5.0);

                // Zoom toward cursor: adjust pan so point under cursor stays fixed
                let cursor_offset = cursor_pos - center - self.pan_offset;
                let zoom_factor = 1.0 - new_zoom / self.zoom;
                self.pan_offset += cursor_offset * zoom_factor;

                self.zoom = new_zoom;
            }
        }

        // Run physics simulation (uses graph-space center, unaffected by viewport pan)
        // Only simulate visible nodes (respects timeline + importance filters)
        // Wire proximity stiffness into layout before step
        self.layout.similarity_stiffness = self.proximity_stiffness;
        let physics_visible = self.compute_physics_visible_nodes();
        let node_sizes = self.compute_node_sizes();
        self.layout.step(&mut self.graph, center, physics_visible.as_ref(), node_sizes.as_ref());

        // Cache values for transform closure to avoid borrowing self
        let pan_offset = self.pan_offset;
        let zoom = self.zoom;

        // Transform helper: graph space -> screen space
        // Pan is in screen space (applied after zoom) for 1:1 movement at any zoom level
        let transform = |pos: Pos2| -> Pos2 {
            let centered = pos.to_vec2() - center.to_vec2();
            center + centered * zoom + pan_offset
        };

        // Ensure effective visible set is fresh (may have been dirtied by sidebar clicks
        // earlier in this frame, after the top-of-frame refresh in update())
        if self.effective_visible_dirty {
            self.rebuild_effective_visible_set();
            // Note: temporal_edges_dirty will be handled in next frame's update()
        }
        let evn = &self.effective_visible_nodes;
        let any_filter = self.any_filter_active();

        // Draw edges first (behind nodes)
        // Per-node neighbor cap for similarity edges (client-side filtering)
        let max_neighbors = self.graph.max_neighbors_per_node;
        let mut sim_degree: HashMap<&str, usize> = HashMap::new();

        for edge in &self.graph.data.edges {
            // Check if edge is dimmed (timeline-hidden) vs fully hidden (other filters)
            let is_timeline_dimmed = self.timeline_enabled && !self.graph.is_edge_visible(edge);

            // Skip edges where either endpoint is not effectively visible
            if any_filter {
                if !evn.contains(&edge.source) || !evn.contains(&edge.target) {
                    continue;
                }
            }

            // Per-node neighbor cap: skip similarity edges once a node hits the limit
            if edge.is_similarity && max_neighbors > 0 {
                let src_deg = sim_degree.get(edge.source.as_str()).copied().unwrap_or(0);
                let tgt_deg = sim_degree.get(edge.target.as_str()).copied().unwrap_or(0);
                if src_deg >= max_neighbors || tgt_deg >= max_neighbors {
                    continue;
                }
                *sim_degree.entry(edge.source.as_str()).or_insert(0) += 1;
                *sim_degree.entry(edge.target.as_str()).or_insert(0) += 1;
            }

            let source_pos = match self.graph.get_pos(&edge.source) {
                Some(p) => transform(p),
                None => continue,
            };
            let target_pos = match self.graph.get_pos(&edge.target) {
                Some(p) => transform(p),
                None => continue,
            };

            let base_opacity = if edge.is_temporal {
                self.temporal_edge_opacity
            } else if edge.is_similarity {
                self.proximity_edge_opacity
            } else {
                0.5
            };

            // Use greyscale and reduced opacity for timeline-dimmed edges
            // For similarity edges, use query-specific color if available
            let base_color = if edge.is_similarity {
                edge.query_index
                    .and_then(|qi| self.proximity_queries.get(qi))
                    .map(|pq| pq.color)
                    .unwrap_or_else(|| self.graph.edge_color(edge))
            } else {
                self.graph.edge_color(edge)
            };
            let mut color = base_color.gamma_multiply(base_opacity);
            if is_timeline_dimmed {
                color = crate::graph::types::to_greyscale(color).gamma_multiply(0.4);
            }
            let stroke = Stroke::new(1.5 * self.zoom, color);

            if edge.is_similarity {
                // Draw dotted line for similarity/proximity edges
                let diff = target_pos - source_pos;
                let length = diff.length();
                let dir = diff / length;
                let dot_len = 4.0 * self.zoom;
                let gap_len = 4.0 * self.zoom;
                let step = dot_len + gap_len;
                let mut d = 0.0;
                while d < length {
                    let seg_end = (d + dot_len).min(length);
                    let p0 = source_pos + dir * d;
                    let p1 = source_pos + dir * seg_end;
                    painter.line_segment([p0, p1], stroke);
                    d += step;
                }
            } else {
                painter.line_segment([source_pos, target_pos], stroke);
            }

            // Draw arrow if enabled
            if self.show_arrows {
                let dir = (target_pos - source_pos).normalized();
                let arrow_size = 8.0 * self.zoom;
                let arrow_pos = target_pos - dir * (self.node_size * self.zoom + 2.0);

                let perp = Vec2::new(-dir.y, dir.x);
                let p1 = arrow_pos;
                let p2 = arrow_pos - dir * arrow_size + perp * arrow_size * 0.5;
                let p3 = arrow_pos - dir * arrow_size - perp * arrow_size * 0.5;

                painter.add(egui::Shape::convex_polygon(
                    vec![p1, p2, p3],
                    color,
                    Stroke::NONE,
                ));
            }
        }

        // Update filtered edge count for UI display
        if max_neighbors > 0 {
            let total: usize = sim_degree.values().sum();
            self.proximity_edge_count_filtered = total / 2; // each edge counted twice
        }

        // Draw bypass edges (bridging over hidden nodes in Inactive mode)
        if !self.bypass_edges.is_empty() {
            for edge in &self.bypass_edges.clone() {
                // Skip bypass edges where either endpoint is not effectively visible
                if any_filter {
                    if !evn.contains(&edge.source) || !evn.contains(&edge.target) {
                        continue;
                    }
                }
                let source_pos = match self.graph.get_pos(&edge.source) {
                    Some(p) => transform(p),
                    None => continue,
                };
                let target_pos = match self.graph.get_pos(&edge.target) {
                    Some(p) => transform(p),
                    None => continue,
                };
                let color = self.graph.edge_color(edge).gamma_multiply(0.5);
                let stroke = Stroke::new(1.5 * self.zoom, color);
                painter.line_segment([source_pos, target_pos], stroke);
            }
        }

        // Detect hover - select closest node to cursor
        // Note: Timeline-dimmed nodes are hoverable (they're greyed out, not hidden)
        let mut new_hovered = None;
        if let Some(hover_pos) = response.hover_pos() {
            let mut closest: Option<(String, f32)> = None; // (node_id, distance)

            for node in &self.graph.data.nodes {
                // Skip nodes not in effective visible set
                if any_filter && !evn.contains(&node.id) {
                    continue;
                }
                if let Some(pos) = self.graph.get_pos(&node.id) {
                    let screen_pos = transform(pos);
                    let distance = screen_pos.distance(hover_pos);

                    if closest.is_none() || distance < closest.as_ref().unwrap().1 {
                        closest = Some((node.id.clone(), distance));
                    }
                }
            }

            new_hovered = closest.map(|(id, _)| id);
        }

        // Hover-to-scrub: move timeline playhead to hovered node's timestamp
        // Same-session nodes switch instantly; cross-session requires click
        let prev_hovered = self.graph.hovered_node.clone();
        self.graph.hovered_node = new_hovered;

        // DISABLED FOR DEBUGGING
        if false && self.hover_scrubs_timeline && self.timeline_enabled {
            if let Some(ref hovered_id) = self.graph.hovered_node {
                // Only instant-scrub for same-session (visible) nodes
                if self.graph.is_node_visible(hovered_id) && self.graph.hovered_node != prev_hovered {
                    let node_time = self.graph.get_node(hovered_id).and_then(|n| n.timestamp_secs());
                    if let Some(t) = node_time {
                        let new_pos = self.graph.timeline.position_at_time(t);
                        self.graph.timeline.position = new_pos.max(self.graph.timeline.start_position);
                        self.graph.update_visible_nodes();
                        self.effective_visible_dirty = true;
                    }
                }
                // Cross-session (dimmed) nodes: do nothing on hover, handled by click
            }
        }

        // Cmd+Hover: compute neighborhood preview while Cmd is held
        let modifiers = ui.input(|i| i.modifiers);
        if modifiers.command {
            if let Some(ref hovered_id) = self.graph.hovered_node {
                let adj = self.build_adjacency_list(self.neighborhood_include_temporal);
                let mut seeds = HashSet::new();
                seeds.insert(hovered_id.clone());
                self.cmd_hover_neighbors = self.expand_to_neighbors(&seeds, self.neighborhood_depth, &adj);
            } else {
                self.cmd_hover_neighbors.clear();
            }
        } else {
            self.cmd_hover_neighbors.clear();
        }
        if !self.cmd_hover_neighbors.is_empty() {
            ui.ctx().request_repaint();
        }

        // Two-pass node rendering:
        // Pass 1: Compute all size multipliers and find max
        // Tuple: (index, multiplier, is_timeline_dimmed, is_same_project_future)
        let mut node_multipliers: Vec<(usize, f32, bool, bool)> = Vec::new();
        let mut max_multiplier: f32 = 0.001; // Avoid division by zero

        for (idx, node) in self.graph.data.nodes.iter().enumerate() {
            // Check if node is timeline-dimmed (visible but greyed out)
            let is_timeline_dimmed = self.timeline_enabled && !self.graph.is_node_visible(&node.id);
            let is_same_project_future = self.is_same_project_future_node(node);

            // Skip nodes not in effective visible set
            if any_filter && !evn.contains(&node.id) {
                continue;
            }

            if self.graph.get_pos(&node.id).is_some() {
                // Unified node sizing formula:
                // size = base * exp(w_imp * importance) * exp(w_tok * tokens_norm) * exp(-w_time * time_dist)

                // 1. Importance factor (0-1, default 0.5)
                let importance = node.importance_score.unwrap_or(0.5);
                let imp_factor = (self.w_importance * importance).exp();

                // 2. Token factor (log-normalized 0-1)
                let tokens_norm = self.graph.normalize_tokens(node);
                let tok_factor = (self.w_tokens * tokens_norm).exp();

                // 3. Time/recency factor (distance from scrubber, 0-1)
                let time_factor = if self.graph.timeline.max_time > self.graph.timeline.min_time {
                    if let Some(node_time) = node.timestamp_secs() {
                        let time_range = self.graph.timeline.max_time - self.graph.timeline.min_time;
                        let scrubber_time = self.graph.timeline.time_at_position(self.graph.timeline.position);
                        let distance = (scrubber_time - node_time).abs();
                        let normalized_distance = (distance / time_range).clamp(0.0, 1.0) as f32;
                        (-self.w_time * normalized_distance).exp()
                    } else {
                        1.0 // No timestamp = neutral
                    }
                } else {
                    1.0 // No time range = neutral
                };

                // Combine factors multiplicatively
                let raw_multiplier = imp_factor * tok_factor * time_factor;

                // Same-project future nodes should be treated as active (not dimmed)
                let is_dimmed_for_rendering = is_timeline_dimmed && !is_same_project_future;

                node_multipliers.push((idx, raw_multiplier, is_dimmed_for_rendering, is_same_project_future));

                // Include non-dimmed nodes AND same-project future nodes in max calculation
                if !is_dimmed_for_rendering {
                    max_multiplier = max_multiplier.max(raw_multiplier);
                }
            }
        }

        // Compute normalization scale: largest visible node gets max_node_multiplier
        let scale = self.max_node_multiplier / max_multiplier;

        // Pass 2: Draw nodes with normalized sizes
        // Draw dimmed nodes first (behind active nodes)
        for &(idx, _raw_multiplier, is_dimmed, is_same_project_future) in &node_multipliers {
            if !is_dimmed {
                continue; // Skip active nodes in this pass
            }
            // Skip same-project future nodes - they'll be rendered in pass 2b
            if is_same_project_future {
                continue;
            }
            let node = &self.graph.data.nodes[idx];
            if let Some(pos) = self.graph.get_pos(&node.id) {
                let screen_pos = transform(pos);

                // Dimmed nodes use a fixed smaller size
                let size = self.node_size * self.zoom * 0.5;

                // Use greyscale color with reduced opacity
                let base_color = self.graph.node_color(node);
                let color = crate::graph::types::to_greyscale(base_color).gamma_multiply(0.4);

                // Draw node
                painter.circle_filled(screen_pos, size, color);

                // Draw inner circle for Claude responses (also greyscale)
                if node.role == crate::graph::types::Role::Assistant {
                    let inner_size = size * 0.4;
                    painter.circle_filled(screen_pos, inner_size, Color32::from_gray(30));
                }

                // Minimal border for dimmed nodes
                painter.circle_stroke(screen_pos, size, Stroke::new(1.0, color.gamma_multiply(0.7)));
            }
        }

        // Pass 2b: Draw active (non-dimmed) nodes on top
        // Get current scrubber time for "future node" desaturation
        let scrubber_time = if self.timeline_enabled {
            Some(self.graph.timeline.time_at_position(self.graph.timeline.position))
        } else {
            None
        };

        for (idx, raw_multiplier, is_dimmed, is_same_project_future) in node_multipliers {
            if is_dimmed {
                continue; // Already drawn in previous pass
            }
            let node = &self.graph.data.nodes[idx];
            if let Some(pos) = self.graph.get_pos(&node.id) {
                let screen_pos = transform(pos);
                let is_hovered = self.graph.hovered_node.as_ref() == Some(&node.id);
                let is_selected = self.graph.selected_node.as_ref() == Some(&node.id);

                // Apply normalization and clamp
                let size_multiplier = (raw_multiplier * scale).clamp(0.05, self.max_node_multiplier);
                let base_size = self.node_size * self.zoom * size_multiplier;
                let size = if is_hovered || is_selected {
                    base_size * 1.3
                } else {
                    base_size
                };

                // Use project or session color based on mode
                let base_color = self.graph.node_color(node);

                // Color logic:
                // - Same-project future nodes: greyscale
                // - Regular future nodes (in session, after scrubber): desaturated
                // - Everything else: full color
                let color = if is_same_project_future {
                    crate::graph::types::to_greyscale(base_color)
                } else {
                    let is_future = scrubber_time
                        .and_then(|st| node.timestamp_secs().map(|nt| nt > st))
                        .unwrap_or(false);
                    if is_future && !is_hovered && !is_selected {
                        crate::graph::types::desaturate(base_color, 0.7)
                    } else {
                        base_color
                    }
                };

                // Apply proximity heat-map overlay when active
                let color = if self.any_proximity_active() {
                    // Get score: either from specific query or max across all
                    let score = match self.proximity_heat_map_index {
                        Some(hmi) => self.proximity_queries.get(hmi)
                            .and_then(|q| q.scores.get(&node.id).copied()),
                        None => self.proximity_queries.iter()
                            .filter(|q| q.active)
                            .filter_map(|q| q.scores.get(&node.id).copied())
                            .reduce(f32::max),
                    };
                    match score {
                        Some(s) => {
                            let grey = crate::graph::types::to_greyscale(color);
                            crate::graph::types::lerp_color(grey.gamma_multiply(0.3), color, s)
                        }
                        None => crate::graph::types::to_greyscale(color).gamma_multiply(0.15),
                    }
                } else {
                    color
                };

                // Draw node differently for same-project future nodes
                if is_same_project_future {
                    // Hollow circle (stroke only, no fill)
                    painter.circle_stroke(screen_pos, size, Stroke::new(3.0, color));
                } else {
                    // Regular filled circle
                    painter.circle_filled(screen_pos, size, color);
                }

                // Draw inner circle for Claude responses
                if node.role == crate::graph::types::Role::Assistant {
                    let inner_size = size * 0.4;
                    if is_same_project_future {
                        // Hollow inner circle for same-project future nodes
                        painter.circle_stroke(screen_pos, inner_size, Stroke::new(2.0, Color32::from_gray(150)));
                    } else {
                        // Filled inner circle for regular nodes
                        painter.circle_filled(screen_pos, inner_size, Color32::BLACK);
                    }
                }

                // Draw border - cyan for summary/cmd-neighbor, yellow for selected, white for hovered
                // Skip border for same-project future nodes (they already have a stroke)
                if !is_same_project_future {
                    let is_summary_node = self.summary_node_id.as_ref() == Some(&node.id);
                    let is_cmd_neighbor = self.cmd_hover_neighbors.contains(&node.id);
                    let border_color = if is_summary_node {
                        theme::state::ACTIVE // Cyan for summary node
                    } else if is_selected {
                        theme::state::SELECTED
                    } else if is_cmd_neighbor {
                        theme::state::ACTIVE // Cyan for cmd-hover neighbor
                    } else if is_hovered {
                        theme::state::HOVER
                    } else {
                        color.gamma_multiply(0.7)
                    };
                    let border_width = if is_summary_node {
                        theme::stroke_width::ACTIVE
                    } else if is_selected || is_hovered {
                        theme::stroke_width::SELECTED
                    } else if is_cmd_neighbor {
                        theme::stroke_width::HOVER
                    } else {
                        theme::stroke_width::NORMAL
                    };
                    painter.circle_stroke(screen_pos, size, Stroke::new(border_width, border_color));
                }
            }
        }

        // Handle click selection with double-click and Ctrl+Click detection
        // Use the already-computed closest node from hover detection
        if response.clicked() {
            let clicked_node = self.graph.hovered_node.clone();
            let modifiers = ui.input(|i| i.modifiers);

            if let Some(ref node_id) = clicked_node {
                // Cross-session click: if clicking on a dimmed (non-visible) node, jump timeline to it
                // DISABLED FOR DEBUGGING
                if false && self.hover_scrubs_timeline && self.timeline_enabled {
                    if !self.graph.is_node_visible(node_id) {
                        let node_time = self.graph.get_node(node_id).and_then(|n| n.timestamp_secs());
                        if let Some(t) = node_time {
                            let new_pos = self.graph.timeline.position_at_time(t);
                            self.graph.timeline.position = new_pos.max(self.graph.timeline.start_position);
                            self.graph.update_visible_nodes();
                            self.effective_visible_dirty = true;
                        }
                    }
                }

                // Ctrl+Click (Cmd+Click on macOS) → neighborhood summary
                if modifiers.command {
                    self.trigger_neighborhood_summary(node_id.clone());
                } else {
                    // Check for double-click (same node within 500ms)
                    let now = Instant::now();
                    let elapsed = now.duration_since(self.last_click_time).as_millis();
                    let same_node = self.last_click_node.as_ref() == Some(node_id);
                    let is_double_click = same_node && elapsed < 500;

                    if is_double_click {
                        self.trigger_summary_for_node(node_id.clone());
                    }

                    self.last_click_time = now;
                    self.last_click_node = clicked_node.clone();
                }
            } else {
                self.last_click_node = None;
            }

            self.graph.selected_node = clicked_node;
        }

        // Draw tooltip for hovered node
        if let Some(ref hovered_id) = self.graph.hovered_node {
            if let Some(node) = self.graph.get_node(hovered_id) {
                if let Some(pos) = self.graph.get_pos(hovered_id) {
                    let screen_pos = transform(pos);
                    let tooltip_pos = screen_pos + Vec2::new(self.node_size * self.zoom + 10.0, 0.0);

                    let mut lines: Vec<String> = Vec::new();

                    if self.debug_tooltip {
                        // Debug tooltip: node classification and rendering info
                        lines.push("DEBUG NODE CLASSIFICATION".to_string());
                        lines.push(String::new());

                        // Session ID
                        lines.push(format!("Session: {}", node.session_id));

                        // Node properties
                        let mut properties = Vec::new();
                        if self.is_after_playhead(node) {
                            properties.push("after playhead");
                        } else {
                            properties.push("before/at playhead");
                        }
                        if self.is_same_session_as_selected(node) {
                            properties.push("same session as selected");
                        } else {
                            properties.push("different session");
                        }
                        if self.is_same_project_as_selected(node) {
                            properties.push("same project as selected");
                        } else {
                            properties.push("different project");
                        }
                        lines.push(format!("Properties: {}", properties.join(", ")));

                        // Display logic
                        let mut display_props = Vec::new();
                        let is_timeline_dimmed = self.timeline_enabled && !self.graph.is_node_visible(&node.id);
                        let is_same_project_future = self.is_same_project_future_node(node);

                        // Hollow vs filled
                        if is_same_project_future {
                            display_props.push("HOLLOW");
                        } else {
                            display_props.push("filled");
                        }
                        // Physics
                        if is_same_project_future {
                            display_props.push("physics enabled");
                        } else if is_timeline_dimmed {
                            display_props.push("no physics");
                        } else {
                            display_props.push("physics enabled");
                        }
                        // Color/saturation
                        if is_same_project_future {
                            display_props.push("greyscale");
                        } else if is_timeline_dimmed {
                            display_props.push("greyscale");
                            display_props.push("40% opacity");
                        } else {
                            let is_future = self.is_after_playhead(node);
                            if is_future {
                                display_props.push("desaturated (70%)");
                            } else {
                                display_props.push("full color");
                            }
                        }
                        // Size
                        if is_timeline_dimmed && !is_same_project_future {
                            display_props.push("0.5x size");
                        } else {
                            display_props.push("variable size");
                        }
                        lines.push(format!("Display: {}", display_props.join(", ")));
                    } else {
                        // Normal tooltip: content preview + metadata
                        // Content preview — word-wrap to ~50 chars, max 4 lines
                        let preview = &node.content_preview;
                        let max_line_len = 50;
                        let max_preview_lines = 4;
                        let mut char_iter = preview.chars().peekable();
                        let mut preview_lines = 0;
                        while char_iter.peek().is_some() && preview_lines < max_preview_lines {
                            let chunk: String = char_iter.by_ref().take(max_line_len).collect();
                            lines.push(chunk.trim_end().to_string());
                            preview_lines += 1;
                        }
                        if char_iter.peek().is_some() {
                            if let Some(last) = lines.last_mut() {
                                last.push_str("...");
                            }
                        }

                        lines.push(String::new());

                        // Project
                        lines.push(format!("Project: {}", node.project));

                        // Timestamp — relative "3 hours ago", "Yesterday at 2:30 PM", etc.
                        if let Some(secs) = node.timestamp_secs() {
                            lines.push(format!("Time: {}", self.graph.timeline.format_time(secs)));
                        }

                        // Tokens — compact "1.2k in / 3.4k out"
                        let in_tok = node.input_tokens.unwrap_or(0);
                        let out_tok = node.output_tokens.unwrap_or(0);
                        if in_tok > 0 || out_tok > 0 {
                            let fmt_tok = |t: i32| -> String {
                                if t >= 1000 { format!("{:.1}k", t as f64 / 1000.0) }
                                else { format!("{}", t) }
                            };
                            lines.push(format!("Tokens: {} in / {} out", fmt_tok(in_tok), fmt_tok(out_tok)));
                        }

                        // Tools used
                        if node.has_tool_usage {
                            lines.push("Tools used".to_string());
                        }
                    }

                    let tooltip_text = lines.join("\n");

                    let galley = painter.layout_no_wrap(
                        tooltip_text,
                        egui::FontId::new(13.0, egui::FontFamily::Proportional),
                        Color32::WHITE,
                    );

                    let tooltip_rect = egui::Rect::from_min_size(
                        tooltip_pos,
                        galley.size() + Vec2::splat(16.0),
                    );

                    painter.rect_filled(
                        tooltip_rect,
                        4.0,
                        Color32::from_rgba_unmultiplied(20, 20, 30, 230),
                    );
                    painter.galley(tooltip_pos + Vec2::splat(8.0), galley, Color32::WHITE);
                }
            }
        }

        // Loading indicator with skeleton animation
        if self.loading {
            // Animated loading pulse
            let time = ui.ctx().input(|i| i.time);
            let pulse = ((time * 2.0).sin() * 0.5 + 0.5) as f32;
            let text_color = Color32::from_rgba_unmultiplied(
                240,
                240,
                245,
                (150.0 + pulse * 105.0) as u8
            );

            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                "Loading...",
                egui::FontId::proportional(24.0),
                text_color,
            );

            // Draw skeleton nodes for preview
            let skeleton_positions = [
                center + Vec2::new(-100.0, -50.0),
                center + Vec2::new(80.0, -30.0),
                center + Vec2::new(-60.0, 60.0),
                center + Vec2::new(120.0, 40.0),
            ];
            for (i, pos) in skeleton_positions.iter().enumerate() {
                let size = 8.0 + (i as f32 * 2.0);
                let phase = ((time * 1.5 + i as f64 * 0.5).sin() * 0.5 + 0.5) as f32;
                let alpha = (100.0 + phase * 80.0) as u8;
                painter.circle_filled(*pos, size, Color32::from_rgba_unmultiplied(80, 85, 100, alpha));
            }

            ui.ctx().request_repaint(); // Keep animating
        }
    }

    /// Render split view with graph on left and histogram on right
    fn render_split_view(&mut self, ui: &mut egui::Ui) {
        let available = ui.available_rect_before_wrap();

        // Calculate split dimensions
        let divider_width = 4.0;
        let graph_width = available.width() * self.histogram_split_ratio - divider_width / 2.0;
        let histogram_width = available.width() * (1.0 - self.histogram_split_ratio) - divider_width / 2.0;

        // Graph panel (left)
        let graph_rect = egui::Rect::from_min_size(
            available.min,
            egui::vec2(graph_width, available.height()),
        );

        // Divider (center)
        let divider_rect = egui::Rect::from_min_size(
            egui::pos2(available.min.x + graph_width, available.min.y),
            egui::vec2(divider_width, available.height()),
        );

        // Histogram panel (right)
        let histogram_rect = egui::Rect::from_min_size(
            egui::pos2(available.min.x + graph_width + divider_width, available.min.y),
            egui::vec2(histogram_width, available.height()),
        );

        // Render graph in left pane
        let mut graph_ui = ui.new_child(egui::UiBuilder::new().max_rect(graph_rect));
        self.render_graph(&mut graph_ui);

        // Render draggable divider
        self.render_divider(ui, divider_rect);

        // Render histogram in right pane
        let mut histogram_ui = ui.new_child(egui::UiBuilder::new().max_rect(histogram_rect));
        self.render_token_histogram(&mut histogram_ui);
    }

    /// Render the draggable divider between graph and histogram
    fn render_divider(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());

        // Handle dragging
        if response.dragged() {
            let drag_delta_x = response.drag_delta().x;
            let available_width = ui.available_width();
            let ratio_delta = drag_delta_x / available_width;
            self.histogram_split_ratio = (self.histogram_split_ratio + ratio_delta).clamp(0.2, 0.8);
            self.histogram_dragging_divider = true;
            self.mark_settings_dirty();
        } else if self.histogram_dragging_divider && !response.is_pointer_button_down_on() {
            self.histogram_dragging_divider = false;
        }

        // Visual feedback
        let color = if response.hovered() || self.histogram_dragging_divider {
            theme::border::FOCUS
        } else {
            theme::border::SUBTLE
        };

        ui.painter().rect_filled(rect, 0.0, color);

        // Change cursor on hover
        if response.hovered() || self.histogram_dragging_divider {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
    }

    /// Get the color for a session in the histogram, matching graph node colors
    fn histogram_session_color(&self, session_id: &str, project: &str) -> egui::Color32 {
        use crate::graph::types::{ColorMode, hsl_to_rgb};
        match self.graph.color_mode {
            ColorMode::Project if !project.is_empty() => {
                let hue = self.graph.project_colors.get(project).copied().unwrap_or(0.0);
                hsl_to_rgb(self.graph.apply_hue_offset(hue), 0.7, 0.55)
            }
            ColorMode::Hybrid if !project.is_empty() => {
                let hue = self.graph.project_colors.get(project).copied().unwrap_or(0.0);
                let t = self.graph.session_position_in_project(session_id, project);
                let sat = 0.5 + t * 0.4;
                let light = 0.65 - t * 0.2;
                hsl_to_rgb(self.graph.apply_hue_offset(hue), sat, light)
            }
            _ => {
                let hue = self.graph.session_colors.get(session_id).copied().unwrap_or(0.0);
                hsl_to_rgb(self.graph.apply_hue_offset(hue), 0.7, 0.5)
            }
        }
    }

    /// Render the token usage histogram
    fn render_token_histogram(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.heading("Token Usage");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::ComboBox::from_id_salt("histogram_stack_order")
                        .selected_text(self.histogram_stack_order.label())
                        .width(120.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.histogram_stack_order, HistogramStackOrder::MostTokens, "Most Tokens");
                            ui.selectable_value(&mut self.histogram_stack_order, HistogramStackOrder::OldestFirst, "Oldest First");
                            ui.selectable_value(&mut self.histogram_stack_order, HistogramStackOrder::MostMessages, "Most Messages");
                        });
                });
            });

            ui.separator();

            let bins = self.aggregate_token_bins();

            if bins.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label("No token data available");
                });
                return;
            }

            self.render_histogram_bars(ui, &bins);
        });
    }

    /// Render the histogram bars using direct painter calls.
    /// Session-colored stacking with grey-out for filtered sessions, click-to-filter, trackpad zoom/pan.
    fn render_histogram_bars(&mut self, ui: &mut egui::Ui, bins: &[TokenBin]) {
        if bins.is_empty() {
            return;
        }

        let max_total = bins.iter()
            .map(|b| b.total_tokens)
            .max()
            .unwrap_or(1);

        let bar_width = self.histogram_bar_width;
        let label_height = 20.0;
        let available_height = (ui.available_height() - label_height).max(40.0);
        let total_width = bins.len() as f32 * bar_width;

        // Allocate one big rect for the entire histogram
        // Use hover sense so trackpad scroll gestures aren't consumed as drags
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(total_width.max(ui.available_width()), available_height + label_height),
            egui::Sense::click(),
        );

        let bar_area = egui::Rect::from_min_size(rect.min, egui::vec2(total_width, available_height));
        let painter = ui.painter_at(rect);

        // Handle trackpad zoom and pan
        let pointer_over = ui.rect_contains_pointer(rect);
        if pointer_over {
            let scroll = ui.input(|i| i.smooth_scroll_delta);
            let raw_scroll = ui.input(|i| i.raw_scroll_delta);
            let modifiers = ui.input(|i| i.modifiers);

            if modifiers.command {
                // Zoom: cmd + scroll Y (pinch gesture)
                let zoom_delta = scroll.y * 0.01;
                self.histogram_bar_width = (self.histogram_bar_width * (1.0 + zoom_delta)).clamp(8.0, 120.0);
            } else {
                // Pan: use horizontal scroll directly (trackpad two-finger swipe),
                // fall back to vertical scroll if no horizontal component
                let pan_smooth = if scroll.x.abs() > 0.5 { scroll.x } else { scroll.y };
                let pan_raw = if raw_scroll.x.abs() > 0.5 { raw_scroll.x } else { raw_scroll.y };
                // Prefer raw scroll (direct trackpad input), fall back to smooth
                let pan = if pan_raw.abs() > 0.1 { pan_raw } else { pan_smooth };
                self.histogram_scroll_offset -= pan;
            }
        }

        // Clamp scroll offset
        let max_scroll = (total_width - rect.width()).max(0.0);
        self.histogram_scroll_offset = self.histogram_scroll_offset.clamp(0.0, max_scroll);

        // Determine which bin is hovered
        let mut hovered_bin_idx: Option<usize> = None;

        if let Some(pointer) = response.hover_pos() {
            let rel_x = pointer.x - rect.min.x + self.histogram_scroll_offset;
            let bin_idx = (rel_x / bar_width) as usize;
            if bin_idx < bins.len() && pointer.y >= rect.min.y && pointer.y <= rect.min.y + available_height {
                hovered_bin_idx = Some(bin_idx);
            }
        }
        self.histogram_hovered_bin = hovered_bin_idx;

        // Track click for click-to-filter
        let clicked = response.clicked();
        let click_pos = response.interact_pointer_pos();
        let mut click_hit_segment = false;

        // Paint bars directly
        for (i, bin) in bins.iter().enumerate() {
            let bar_x = rect.min.x + i as f32 * bar_width - self.histogram_scroll_offset;

            // Skip bars outside visible area
            if bar_x + bar_width < rect.min.x || bar_x > rect.max.x {
                continue;
            }

            if bin.total_tokens > 0 {
                let scale = available_height / max_total as f32;
                let mut y_offset = 0.0;

                // Draw session segments bottom to top
                for session in &bin.sessions {
                    let height = session.total_tokens as f32 * scale;
                    let seg_rect = egui::Rect::from_min_size(
                        egui::pos2(bar_x, bar_area.max.y - y_offset - height),
                        egui::vec2(bar_width, height),
                    );
                    let base_color = self.histogram_session_color(&session.session_id, &session.project);
                    let color = if session.is_filtered {
                        // Grey-out filtered sessions
                        let grey = (base_color.r() as f32 * 0.299
                            + base_color.g() as f32 * 0.587
                            + base_color.b() as f32 * 0.114) as u8;
                        Color32::from_rgba_unmultiplied(grey, grey, grey, 100)
                    } else {
                        base_color
                    };
                    painter.rect_filled(seg_rect, 0.0, color);

                    // Progressive drill-down on click:
                    // Same segment repeatedly: project → session → clear
                    // Different segment: toggle that project
                    if clicked && !session.project.is_empty() {
                        if let Some(pos) = click_pos {
                            if seg_rect.contains(pos) {
                                click_hit_segment = true;
                                let this_seg = (session.session_id.clone(), session.project.clone());
                                let same_as_last = self.histogram_last_clicked.as_ref() == Some(&this_seg);

                                if same_as_last {
                                    // Consecutive click on same segment — advance drill
                                    match self.histogram_drill_level {
                                        1 => {
                                            // Drill to session
                                            self.histogram_session_filter = Some(session.session_id.clone());
                                            self.histogram_drill_level = 2;
                                            self.effective_visible_dirty = true;
                                        }
                                        2 => {
                                            // Clear all filters
                                            self.project_filter = FilterMode::Off;
                                            self.selected_projects = self.available_projects.iter().cloned().collect();
                                            self.histogram_session_filter = None;
                                            self.histogram_last_clicked = None;
                                            self.histogram_drill_level = 0;
                                            self.effective_visible_dirty = true;
                                        }
                                        _ => {}
                                    }
                                } else {
                                    // Different segment (or first click)
                                    if !self.project_filter.is_active() {
                                        // First filter action: select only this project
                                        self.project_filter = FilterMode::Filtered;
                                        self.selected_projects.clear();
                                        self.selected_projects.insert(session.project.clone());
                                    } else {
                                        // Toggle this project
                                        if self.selected_projects.contains(&session.project) {
                                            self.selected_projects.remove(&session.project);
                                        } else {
                                            self.selected_projects.insert(session.project.clone());
                                        }
                                    }
                                    // Reset drill tracking to this segment
                                    self.histogram_session_filter = None;
                                    self.histogram_last_clicked = Some(this_seg);
                                    self.histogram_drill_level = 1;
                                    self.effective_visible_dirty = true;
                                }
                            }
                        }
                    }

                    y_offset += height;
                }
            }
        }

        // Click on empty space clears all filters
        if clicked && !click_hit_segment && (self.project_filter.is_active() || self.histogram_session_filter.is_some()) {
            self.project_filter = FilterMode::Off;
            self.selected_projects = self.available_projects.iter().cloned().collect();
            self.histogram_session_filter = None;
            self.histogram_last_clicked = None;
            self.histogram_drill_level = 0;
            self.effective_visible_dirty = true;
        }

        // Date labels: tick marks at regular intervals
        let label_interval = ((60.0 / bar_width).ceil() as usize).max(1);
        for (i, bin) in bins.iter().enumerate() {
            if i % label_interval != 0 {
                continue;
            }
            let bar_x = rect.min.x + i as f32 * bar_width - self.histogram_scroll_offset;
            if bar_x < rect.min.x - bar_width || bar_x > rect.max.x + bar_width {
                continue;
            }

            let tick_top = rect.min.y + available_height;
            let tick_bottom = tick_top + 4.0;
            let label_color = theme::text::SECONDARY;

            // Tick mark
            painter.line_segment(
                [egui::pos2(bar_x, tick_top), egui::pos2(bar_x, tick_bottom)],
                egui::Stroke::new(1.0, label_color),
            );

            // Label
            let label = format_timestamp(&bin.timestamp_start);
            painter.text(
                egui::pos2(bar_x + 2.0, tick_bottom + 1.0),
                egui::Align2::LEFT_TOP,
                &label,
                egui::FontId::proportional(10.0),
                label_color,
            );
        }

        // Hover tooltip
        if let Some(idx) = hovered_bin_idx {
            let bin = &bins[idx];
            let bar_x = rect.min.x + idx as f32 * bar_width - self.histogram_scroll_offset;
            let tooltip_rect = egui::Rect::from_min_size(
                egui::pos2(bar_x, rect.min.y),
                egui::vec2(bar_width, available_height),
            );
            // Show hover highlight
            painter.rect_filled(
                tooltip_rect,
                0.0,
                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 15),
            );

            egui::show_tooltip_at_pointer(ui.ctx(), egui::LayerId::new(egui::Order::Tooltip, ui.id().with("hist_layer")), ui.id().with("hist_tooltip"), |ui| {
                ui.label(format!("{} - {}",
                    format_timestamp(&bin.timestamp_start),
                    format_timestamp(&bin.timestamp_end)
                ));
                ui.separator();
                for session in &bin.sessions {
                    let color = self.histogram_session_color(&session.session_id, &session.project);
                    let label = if session.project.is_empty() {
                        format!("{}: {} tokens", &session.session_id[..8.min(session.session_id.len())], session.total_tokens)
                    } else {
                        format!("{}{}: {} tokens",
                            session.project,
                            if session.is_filtered { " (filtered)" } else { "" },
                            session.total_tokens)
                    };
                    ui.colored_label(color, label);
                }
                ui.separator();
                ui.label(format!("Total: {} tokens", bin.total_tokens));
            });
        }
    }

    /// Aggregate token usage into time bins by session.
    /// Includes ALL sessions (not filtered by project), but tags them as filtered.
    /// Sorts session stacking by histogram_stack_order.
    fn aggregate_token_bins(&self) -> Vec<TokenBin> {
        use chrono::{DateTime, Utc};

        // Collect all nodes with token data and valid timestamps
        // Include ALL nodes regardless of project filter (only skip for timeline)
        let mut timestamped_nodes: Vec<_> = self.graph.data.nodes.iter()
            .filter_map(|node| {
                // Skip nodes hidden by timeline
                if self.timeline_enabled && !self.graph.timeline.visible_nodes.contains(&node.id) {
                    return None;
                }

                let ts = node.timestamp.as_ref()?;
                let total = node.input_tokens.unwrap_or(0)
                    + node.output_tokens.unwrap_or(0)
                    + node.cache_read_tokens.unwrap_or(0)
                    + node.cache_creation_tokens.unwrap_or(0);

                if total == 0 {
                    return None;
                }

                Some((ts.clone(), node.session_id.clone(), node.project.clone(), total as i64))
            })
            .collect();

        if timestamped_nodes.is_empty() {
            return Vec::new();
        }

        // Sort by timestamp
        timestamped_nodes.sort_by(|a, b| a.0.cmp(&b.0));

        // Parse timestamps
        let parsed_nodes: Vec<_> = timestamped_nodes.iter()
            .filter_map(|(ts, session_id, project, total)| {
                let parsed = DateTime::parse_from_rfc3339(ts).ok()?.with_timezone(&Utc);
                Some((parsed, session_id.clone(), project.clone(), *total))
            })
            .collect();

        if parsed_nodes.is_empty() {
            return Vec::new();
        }

        // When timeline scrubber is active, synchronize histogram to the visible window
        let (start_time, bin_duration_secs, bin_count) = if self.timeline_enabled {
            let scrubber_start_epoch = self.graph.timeline.time_at_position(self.graph.timeline.start_position);
            let scrubber_end_epoch = self.graph.timeline.time_at_position(self.graph.timeline.position);
            let visible_range_secs = (scrubber_end_epoch - scrubber_start_epoch).max(1.0);

            let start_dt = DateTime::<Utc>::from_timestamp(scrubber_start_epoch as i64, 0)
                .unwrap_or_else(|| parsed_nodes.first().unwrap().0);

            let raw_bin = visible_range_secs / 20.0;
            let bin_dur = if raw_bin <= 60.0 { 60 }
                else if raw_bin <= 5.0 * 60.0 { 5 * 60 }
                else if raw_bin <= 15.0 * 60.0 { 15 * 60 }
                else if raw_bin <= 30.0 * 60.0 { 30 * 60 }
                else if raw_bin <= 3600.0 { 3600 }
                else if raw_bin <= 3.0 * 3600.0 { 3 * 3600 }
                else if raw_bin <= 6.0 * 3600.0 { 6 * 3600 }
                else if raw_bin <= 12.0 * 3600.0 { 12 * 3600 }
                else if raw_bin <= 86400.0 { 86400 }
                else { (7 * 86400_i64).max(raw_bin as i64) };

            let count = ((visible_range_secs / bin_dur as f64).ceil() as usize).max(1);
            (start_dt, bin_dur, count)
        } else {
            let bin_dur = bin_duration_for_hours(self.time_range_hours) as i64;
            let data_start = parsed_nodes.first().unwrap().0;
            let data_end = parsed_nodes.last().unwrap().0;
            let range_secs = (data_end - data_start).num_seconds();
            let count = ((range_secs as f64 / bin_dur as f64).ceil() as usize).max(1);
            (data_start, bin_dur, count)
        };

        // Initialize bins with session-level tracking
        let mut bin_session_maps: Vec<HashMap<String, (String, i64)>> = Vec::new();
        let mut bins = Vec::new();
        for i in 0..bin_count {
            let bin_start = start_time + chrono::Duration::seconds(i as i64 * bin_duration_secs);
            let bin_end = start_time + chrono::Duration::seconds((i as i64 + 1) * bin_duration_secs);

            bins.push(TokenBin {
                timestamp_start: bin_start.to_rfc3339(),
                timestamp_end: bin_end.to_rfc3339(),
                sessions: Vec::new(),
                total_tokens: 0,
            });
            bin_session_maps.push(HashMap::new());
        }

        // Aggregate nodes into bins by session
        for (timestamp, session_id, project, total) in parsed_nodes {
            let offset = (timestamp - start_time).num_seconds();
            if offset < 0 { continue; }
            let bin_index = (offset / bin_duration_secs) as usize;
            if bin_index < bins.len() {
                let entry = bin_session_maps[bin_index]
                    .entry(session_id.clone())
                    .or_insert_with(|| (project.clone(), 0));
                entry.1 += total;
            }
        }

        // Convert session maps into sorted SessionTokens vecs
        let session_cache = &self.session_metadata_cache;
        let project_filter_active = self.project_filter.is_active();
        let selected_projects = &self.selected_projects;
        let session_filter = &self.histogram_session_filter;
        let stack_order = self.histogram_stack_order;

        for (i, session_map) in bin_session_maps.into_iter().enumerate() {
            let mut sessions: Vec<SessionTokens> = session_map
                .into_iter()
                .map(|(session_id, (project, total))| {
                    let is_filtered = (project_filter_active && !selected_projects.contains(&project))
                        || session_filter.as_ref().is_some_and(|sf| sf != &session_id);
                    SessionTokens {
                        session_id,
                        project,
                        total_tokens: total,
                        is_filtered,
                    }
                })
                .collect();

            // Sort by stack order (bottom of bar = first in vec)
            match stack_order {
                HistogramStackOrder::MostTokens => {
                    sessions.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));
                }
                HistogramStackOrder::OldestFirst => {
                    sessions.sort_by(|a, b| {
                        let a_ts = session_cache.get(&a.session_id).map(|c| c.0).unwrap_or(f64::MAX);
                        let b_ts = session_cache.get(&b.session_id).map(|c| c.0).unwrap_or(f64::MAX);
                        a_ts.partial_cmp(&b_ts).unwrap_or(std::cmp::Ordering::Equal)
                    });
                }
                HistogramStackOrder::MostMessages => {
                    sessions.sort_by(|a, b| {
                        let a_count = session_cache.get(&a.session_id).map(|c| c.1).unwrap_or(0);
                        let b_count = session_cache.get(&b.session_id).map(|c| c.1).unwrap_or(0);
                        b_count.cmp(&a_count)
                    });
                }
            }

            bins[i].total_tokens = sessions.iter().map(|s| s.total_tokens).sum();
            bins[i].sessions = sessions;
        }

        bins
    }

    fn render_timeline(&mut self, ui: &mut egui::Ui) {
        if self.graph.timeline.timestamps.is_empty() {
            ui.label("No timestamped nodes");
            return;
        }

        // Cache values we need before any closures
        let is_playing = self.graph.timeline.playing;
        let current_speed = self.graph.timeline.speed;
        let start_pos = self.graph.timeline.start_position;
        let end_pos = self.graph.timeline.position;
        let visible_count = self.effective_visible_count;
        let total_count = self.graph.data.nodes.len();
        let start_time = self.graph.timeline.time_at_position(start_pos);
        let end_time = self.graph.timeline.time_at_position(end_pos);
        let start_time_str = self.graph.timeline.format_time(start_time);
        let end_time_str = self.graph.timeline.format_time(end_time);
        let timestamps: Vec<f64> = self.graph.timeline.timestamps.clone();
        let min_time = self.graph.timeline.min_time;
        let max_time = self.graph.timeline.max_time;
        let histogram_mode = self.timeline_histogram_mode;
        let bin_duration = bin_duration_for_hours(self.time_range_hours);

        // Helper to calculate position from time
        let position_at_time = |t: f64| -> f32 {
            if max_time <= min_time {
                1.0
            } else {
                ((t - min_time) / (max_time - min_time)) as f32
            }
        };

        ui.horizontal(|ui| {
            // Playback controls
            if is_playing {
                if ui.button("⏸").clicked() {
                    self.graph.timeline.playing = false;
                }
            } else {
                if ui.button("▶").clicked() {
                    self.graph.timeline.playing = true;
                    self.last_playback_time = Instant::now();
                }
            }

            if ui.button("⏮").clicked() {
                self.graph.timeline.position = 0.0;
                self.graph.timeline.start_position = 0.0;
                self.graph.update_visible_nodes();
                self.effective_visible_dirty = true;
            }

            if ui.button("⏭").clicked() {
                self.graph.timeline.position = 1.0;
                self.graph.update_visible_nodes();
                self.effective_visible_dirty = true;
            }

            ui.separator();

            // Speed selector
            ui.label("Speed:");
            let speeds = [0.5, 1.0, 2.0, 4.0, 8.0];
            for speed in speeds {
                let label = format!("{:.0}x", speed);
                if ui.selectable_label(
                    (current_speed - speed).abs() < 0.01,
                    &label
                ).clicked() {
                    self.graph.timeline.speed = speed;
                }
            }

            ui.separator();

            // View mode toggle (notch vs histogram)
            let view_label = if histogram_mode { "📊" } else { "┃┃" };
            let view_tooltip = if histogram_mode { "Histogram view (click for notches)" } else { "Notch view (click for histogram)" };
            if ui.button(view_label).on_hover_text(view_tooltip).clicked() {
                self.timeline_histogram_mode = !self.timeline_histogram_mode;
            }
        });

        ui.add_space(4.0);

        // Time display
        ui.horizontal(|ui| {
            ui.label(format!("Showing: {} → {}", start_time_str, end_time_str));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!("{} / {} nodes", visible_count, total_count));
            });
        });

        ui.add_space(4.0);

        // Main scrubber track
        let (response, painter) = ui.allocate_painter(
            Vec2::new(ui.available_width(), 40.0),
            egui::Sense::click_and_drag()
        );
        let rect = response.rect;

        // Draw track background
        painter.rect_filled(
            rect,
            4.0,
            theme::bg::TIMELINE_TRACK
        );

        // Draw either notches or histogram based on mode
        if histogram_mode {
            // Histogram mode: bin timestamps and draw bars
            let time_span = max_time - min_time;
            if time_span > 0.0 && bin_duration > 0.0 {
                let num_bins = ((time_span / bin_duration).ceil() as usize).max(1);
                let mut bin_counts: Vec<usize> = vec![0; num_bins];

                // Count messages per bin
                for &t in &timestamps {
                    let bin_idx = ((t - min_time) / bin_duration) as usize;
                    let bin_idx = bin_idx.min(num_bins - 1);
                    bin_counts[bin_idx] += 1;
                }

                // Find max count for normalization
                let max_count = bin_counts.iter().copied().max().unwrap_or(1).max(1);

                // Draw histogram bars
                let bar_color = theme::timeline::BAR_INACTIVE;
                let bar_highlight = theme::timeline::BAR_HIGHLIGHT;
                let track_height = rect.height() - 10.0; // Leave padding

                for (i, &count) in bin_counts.iter().enumerate() {
                    if count == 0 {
                        continue;
                    }

                    let bin_start_time = min_time + (i as f64) * bin_duration;
                    let bin_end_time = (bin_start_time + bin_duration).min(max_time);

                    let x_start = rect.left() + position_at_time(bin_start_time) * rect.width();
                    let x_end = rect.left() + position_at_time(bin_end_time) * rect.width();
                    let bar_width = (x_end - x_start - 2.0).max(2.0); // Min 2px, 1px gap

                    let height_ratio = (count as f32) / (max_count as f32);
                    let bar_height = height_ratio * track_height;

                    let bar_rect = egui::Rect::from_min_size(
                        Pos2::new(x_start + 1.0, rect.bottom() - 5.0 - bar_height),
                        Vec2::new(bar_width, bar_height),
                    );

                    // Highlight bars in selected range
                    let bar_in_range = position_at_time(bin_start_time) >= start_pos
                        && position_at_time(bin_end_time) <= end_pos;
                    let color = if bar_in_range { bar_highlight } else { bar_color };

                    painter.rect_filled(bar_rect, 1.0, color);
                }
            }
        } else {
            // Notch mode: draw individual lines for each timestamp
            let notch_color = theme::timeline::NOTCH;
            for &t in &timestamps {
                let pos = position_at_time(t);
                let x = rect.left() + pos * rect.width();
                painter.line_segment(
                    [Pos2::new(x, rect.top() + 5.0), Pos2::new(x, rect.bottom() - 5.0)],
                    Stroke::new(1.0, notch_color)
                );
            }
        }

        // Draw selected range
        let start_x = rect.left() + start_pos * rect.width();
        let end_x = rect.left() + end_pos * rect.width();
        let range_rect = egui::Rect::from_min_max(
            Pos2::new(start_x, rect.top() + 2.0),
            Pos2::new(end_x, rect.bottom() - 2.0)
        );
        painter.rect_filled(
            range_rect,
            2.0,
            theme::accent::orange_subtle()
        );

        // Draw start handle
        let handle_width = 8.0;
        let start_handle_rect = egui::Rect::from_center_size(
            Pos2::new(start_x, rect.center().y),
            Vec2::new(handle_width, rect.height() - 4.0)
        );
        painter.rect_filled(start_handle_rect, 2.0, theme::timeline::HANDLE_START);

        // Draw end/position handle (main scrubber)
        let end_handle_rect = egui::Rect::from_center_size(
            Pos2::new(end_x, rect.center().y),
            Vec2::new(handle_width, rect.height() - 4.0)
        );
        painter.rect_filled(end_handle_rect, 2.0, theme::timeline::HANDLE_END);

        // Handle interaction
        if response.dragged() {
            if let Some(pos) = response.interact_pointer_pos() {
                let new_pos = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);

                // Determine which handle to move based on which is closer
                let dist_to_start = (pos.x - start_x).abs();
                let dist_to_end = (pos.x - end_x).abs();

                if dist_to_start < dist_to_end && dist_to_start < 20.0 {
                    // Move start handle
                    self.graph.timeline.start_position = new_pos.min(self.graph.timeline.position - 0.01);
                } else {
                    // Move end handle (main position)
                    // Snap to nearest notch for smooth scrubbing
                    let snapped = self.graph.timeline.snap_to_notch(new_pos);
                    self.graph.timeline.position = snapped.max(self.graph.timeline.start_position + 0.01);
                }

                self.graph.update_visible_nodes();
                self.effective_visible_dirty = true;
                self.timeline_dragging = true;
            }
        } else {
            self.timeline_dragging = false;
        }

        // Handle click to jump
        if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let new_pos = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                let snapped = self.graph.timeline.snap_to_notch(new_pos);
                self.graph.timeline.position = snapped.max(self.graph.timeline.start_position + 0.01);
                self.graph.update_visible_nodes();
                self.effective_visible_dirty = true;
            }
        }
    }
}

impl eframe::App for DashboardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_fps();
        self.maybe_save_settings();

        // Handle keyboard shortcuts for panel toggles
        // Only trigger when no text input is focused
        ctx.input(|i| {
            if !i.focused {
                if i.key_pressed(egui::Key::B) {
                    self.beads_panel_open = !self.beads_panel_open;
                    self.mark_settings_dirty();
                }
                if i.key_pressed(egui::Key::M) {
                    self.mail_panel_open = !self.mail_panel_open;
                    self.mark_settings_dirty();
                }
            }
        });

        // Check for .beads/ changes and auto-refresh if needed
        if self.check_beads_changed() && !self.loading {
            self.load_graph();
        }

        // Refresh semantic filter cache once per frame if needed
        if self.semantic_filter_cache.is_none() && self.has_active_semantic_filters() {
            self.semantic_filter_cache = self.compute_semantic_filter_visible_set();
        }

        // Rebuild effective visible set when any filter changed
        if self.effective_visible_dirty {
            eprintln!("[rebuild] effective_visible_dirty=true, cache={}, has_active={}, modes={:?}",
                if self.semantic_filter_cache.is_some() { "Some" } else { "None" },
                self.has_active_semantic_filters(),
                self.semantic_filter_modes.iter().map(|(id, m)| (*id, *m)).collect::<Vec<_>>());
            self.rebuild_effective_visible_set();
            self.temporal_edges_dirty = true; // visible set changed → edges need rebuild
        }

        // Rebuild temporal edges when needed (visible set changed, or edge settings changed)
        // Skip during timeline playback to avoid expensive per-frame rebuilds;
        // edges will rebuild when playback stops or is paused.
        if self.temporal_edges_dirty && !self.graph.timeline.playing {
            if self.graph.temporal_attraction_enabled {
                let vis = if self.any_filter_active() {
                    Some(self.effective_visible_nodes.clone())
                } else {
                    None
                };
                self.graph.build_temporal_edges_filtered(vis.as_ref());
            } else {
                // Temporal disabled — just remove temporal edges
                self.graph.data.edges.retain(|e| !e.is_temporal);
            }
            self.temporal_edges_dirty = false;
        }

        // Check for point-in-time summary result from background thread
        if let Some(ref rx) = self.summary_receiver {
            match rx.try_recv() {
                Ok(Ok(data)) => {
                    // Cache for tooltip display
                    if let Some(ref node_id) = self.summary_node_id {
                        self.point_in_time_summary_cache.insert(node_id.clone(), data.clone());
                    }
                    self.summary_data = Some(data);
                    self.summary_loading = false;
                    self.summary_receiver = None;
                }
                Ok(Err(e)) => {
                    self.summary_error = Some(e);
                    self.summary_loading = false;
                    self.summary_receiver = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // Still loading, request repaint to check again
                    ctx.request_repaint();
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.summary_error = Some("Summary request cancelled".to_string());
                    self.summary_loading = false;
                    self.summary_receiver = None;
                }
            }
        }

        // Check for session summary result from background thread
        if let Some(ref rx) = self.session_summary_receiver {
            match rx.try_recv() {
                Ok(Ok(data)) => {
                    // Cache for tooltip display (if we have a valid summary)
                    if data.exists {
                        if let Some(ref session_id) = self.summary_session_id {
                            self.session_summary_cache.insert(session_id.clone(), data.clone());
                        }
                    }
                    self.session_summary_data = Some(data);
                    self.session_summary_loading = false;
                    self.session_summary_receiver = None;
                }
                Ok(Err(e)) => {
                    self.session_summary_loading = false;
                    self.session_summary_receiver = None;
                    self.summary_error = Some(format!("Session summary: {}", e));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // Still loading, request repaint to check again
                    ctx.request_repaint();
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.session_summary_loading = false;
                    self.session_summary_receiver = None;
                    self.summary_error = Some("Session summary fetch disconnected".to_string());
                }
            }
        }

        // Check for neighborhood summary result from background thread
        if let Some(ref rx) = self.neighborhood_summary_receiver {
            match rx.try_recv() {
                Ok(Ok(data)) => {
                    self.neighborhood_summary_data = Some(data);
                    self.neighborhood_summary_loading = false;
                    self.neighborhood_summary_receiver = None;
                }
                Ok(Err(e)) => {
                    self.neighborhood_summary_error = Some(e);
                    self.neighborhood_summary_loading = false;
                    self.neighborhood_summary_receiver = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint();
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.neighborhood_summary_error = Some("Neighborhood summary fetch disconnected".to_string());
                    self.neighborhood_summary_loading = false;
                    self.neighborhood_summary_receiver = None;
                }
            }
        }

        // Drain categorization progress updates
        if let Some(ref rx) = self.categorization_progress_rx {
            while let Ok(status) = rx.try_recv() {
                self.categorization_progress = Some((status.scored, status.total));
            }
        }

        // Check for categorization result from background thread
        if let Some(ref rx) = self.categorization_receiver {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    // Categorization completed - stop polling thread and reload
                    if let Some(ref flag) = self.categorization_done_flag {
                        flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    self.categorizing_filter_id = None;
                    self.categorization_receiver = None;
                    self.categorization_progress_rx = None;
                    self.categorization_progress = None;
                    self.categorization_done_flag = None;
                    self.load_semantic_filters();
                    // Also reload graph to get updated semantic_filter_matches
                    self.load_graph();
                }
                Ok(Err(e)) => {
                    eprintln!("Categorization failed: {}", e);
                    if let Some(ref flag) = self.categorization_done_flag {
                        flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    self.categorizing_filter_id = None;
                    self.categorization_receiver = None;
                    self.categorization_progress_rx = None;
                    self.categorization_progress = None;
                    self.categorization_done_flag = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // Still loading, request repaint to check again
                    ctx.request_repaint();
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if let Some(ref flag) = self.categorization_done_flag {
                        flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    self.categorizing_filter_id = None;
                    self.categorization_receiver = None;
                    self.categorization_progress_rx = None;
                    self.categorization_progress = None;
                    self.categorization_done_flag = None;
                }
            }
        }

        // Check for rescore events from background thread
        if let Some(ref rx) = self.rescore_receiver {
            // Drain all available events (multiple progress updates may be pending)
            loop {
                match rx.try_recv() {
                    Ok(RescoreEvent::Progress(progress)) => {
                        // Update progress display
                        self.rescore_progress = Some(progress);
                        ctx.request_repaint();
                    }
                    Ok(RescoreEvent::Complete(result)) => {
                        // Rescore completed - store result and reload graph
                        self.rescore_result = Some(result);
                        self.rescore_loading = false;
                        self.rescore_progress = None;
                        self.rescore_receiver = None;
                        // Reload graph to get updated importance scores
                        self.load_graph();
                        break;
                    }
                    Ok(RescoreEvent::Error(e)) => {
                        eprintln!("Rescore failed: {}", e);
                        self.rescore_loading = false;
                        self.rescore_progress = None;
                        self.rescore_receiver = None;
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        // No more events, request repaint to check again
                        ctx.request_repaint();
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        self.rescore_loading = false;
                        self.rescore_progress = None;
                        self.rescore_receiver = None;
                        break;
                    }
                }
            }
        }

        // Check for ingest result from background thread
        if let Some(ref rx) = self.ingest_receiver {
            match rx.try_recv() {
                Ok(Ok(result)) => {
                    self.ingest_result = Some(result);
                    self.ingest_loading = false;
                    self.ingest_receiver = None;
                    // Reload graph to show newly ingested data
                    self.load_graph();
                }
                Ok(Err(e)) => {
                    eprintln!("Ingest failed: {}", e);
                    self.ingest_loading = false;
                    self.ingest_receiver = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint();
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.ingest_loading = false;
                    self.ingest_receiver = None;
                }
            }
        }

        // Check for proximity edges + scores results from background threads (per-query)
        let mut any_proximity_completed = false;
        for i in 0..self.proximity_queries.len() {
            if self.proximity_queries[i].rx.is_none() { continue; }
            // Take the rx to avoid borrow conflicts
            let rx = self.proximity_queries[i].rx.take().unwrap();
            match rx.try_recv() {
                Ok(Ok((edges, scores))) => {
                    let count = edges.len();
                    eprintln!("Query '{}': loaded {} edges, {} scored nodes",
                        self.proximity_queries[i].query, count, scores.len());
                    self.proximity_queries[i].edges = edges;
                    self.proximity_queries[i].scores = scores;
                    self.proximity_queries[i].edge_count = count;
                    self.proximity_queries[i].active = true;
                    self.proximity_queries[i].loading = false;
                    any_proximity_completed = true;
                }
                Ok(Err(e)) => {
                    eprintln!("Proximity fetch failed for '{}': {}", self.proximity_queries[i].query, e);
                    self.proximity_queries[i].loading = false;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // Put rx back, still waiting
                    self.proximity_queries[i].rx = Some(rx);
                    ctx.request_repaint();
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.proximity_queries[i].loading = false;
                }
            }
        }
        if any_proximity_completed {
            self.rebuild_all_proximity_edges();
        }

        // Check for embedding generation result from background thread
        if let Some(ref rx) = self.embedding_gen_receiver {
            match rx.try_recv() {
                Ok(Ok(result)) => {
                    eprintln!("Generated {} embeddings", result.generated);
                    self.embedding_gen_loading = false;
                    self.embedding_gen_receiver = None;
                    // Refresh stats
                    self.load_embedding_stats();
                }
                Ok(Err(e)) => {
                    eprintln!("Embedding generation failed: {}", e);
                    self.embedding_gen_loading = false;
                    self.embedding_gen_receiver = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint();
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.embedding_gen_loading = false;
                    self.embedding_gen_receiver = None;
                }
            }
        }


        // Handle playback
        if self.graph.timeline.playing && !self.timeline_dragging {
            let now = Instant::now();
            let delta = now.duration_since(self.last_playback_time).as_secs_f32();
            self.last_playback_time = now;

            // Advance position based on speed
            // At 1x speed, traverse the entire timeline in ~10 seconds
            let advance = delta * self.graph.timeline.speed * 0.1;
            self.graph.timeline.position = (self.graph.timeline.position + advance).min(1.0);
            self.graph.update_visible_nodes();
            self.effective_visible_dirty = true;

            if self.graph.timeline.position >= 1.0 {
                self.graph.timeline.playing = false;
            }
        }

        // Request continuous repaint for physics simulation or playback
        let physics_visible = self.compute_physics_visible_nodes();
        if (self.graph.physics_enabled && !self.layout.is_settled(&self.graph, physics_visible.as_ref()))
            || self.graph.timeline.playing
        {
            ctx.request_repaint();
        }

        // Dark theme
        ctx.set_visuals(egui::Visuals::dark());

        // Floating summary/neighborhood windows (rendered before panels so they float on top)
        self.render_summary_window(ctx);
        self.render_neighborhood_window(ctx);
        self.render_edge_popups(ctx);

        // Sidebar
        egui::SidePanel::left("sidebar")
            .min_width(220.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.render_sidebar(ui);
                });
            });

        // Top panel for hovered node session ID and project
        if let Some(ref hovered_id) = self.graph.hovered_node {
            if let Some(node) = self.graph.data.nodes.iter().find(|n| &n.id == hovered_id) {
                egui::TopBottomPanel::top("session_id_display")
                    .frame(egui::Frame::none()
                        .fill(theme::bg::PANEL)
                        .inner_margin(egui::Margin::symmetric(12.0, 8.0)))
                    .show(ctx, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.label(
                                egui::RichText::new(format!("Session: {} | Project: {}", node.session_id, node.project))
                                    .size(14.0)
                                    .color(theme::text::SECONDARY)
                            );
                        });
                    });
            }
        }

        // Bottom timeline panel (only when enabled)
        if self.timeline_enabled {
            egui::TopBottomPanel::bottom("timeline")
                .min_height(80.0)
                .frame(egui::Frame::none()
                    .fill(theme::bg::PANEL)
                    .inner_margin(egui::Margin::symmetric(12.0, 8.0)))
                .show(ctx, |ui| {
                    self.render_timeline(ui);
                });
        }

        // Beads panel (right side, toggled with B)
        if self.beads_panel_open {
            egui::SidePanel::right("beads_panel")
                .min_width(280.0)
                .max_width(400.0)
                .frame(egui::Frame::none()
                    .fill(theme::bg::PANEL)
                    .inner_margin(egui::Margin::same(12.0)))
                .show(ctx, |ui| {
                    self.render_beads_panel(ui);
                });
        }

        // Mail panel (right side, toggled with M)
        if self.mail_panel_open {
            egui::SidePanel::right("mail_panel")
                .min_width(280.0)
                .max_width(400.0)
                .frame(egui::Frame::none()
                    .fill(theme::bg::PANEL)
                    .inner_margin(egui::Margin::same(12.0)))
                .show(ctx, |ui| {
                    self.render_mail_panel(ui);
                });
        }

        // Main graph area
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::bg::GRAPH))
            .show(ctx, |ui| {
                if !self.db_connected || (!self.loading && self.graph.data.nodes.is_empty()) {
                    self.render_empty_state(ui);
                } else if self.histogram_panel_enabled {
                    self.render_split_view(ui);
                } else {
                    self.render_graph(ui);
                }
            });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Force save settings on exit
        if self.settings_dirty {
            self.sync_settings_from_ui();
            self.settings.save();
        }
    }
}

/// Build adjacency list from graph edges, optionally excluding temporal edges.
fn build_adjacency_list(edges: &[crate::graph::types::GraphEdge], include_temporal: bool) -> HashMap<String, Vec<String>> {
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    for edge in edges {
        if !include_temporal && edge.is_temporal {
            continue;
        }
        adj.entry(edge.source.clone()).or_default().push(edge.target.clone());
        adj.entry(edge.target.clone()).or_default().push(edge.source.clone());
    }
    adj
}

/// BFS expansion from seed nodes to the given depth.
fn expand_to_neighbors(seeds: &HashSet<String>, depth: usize, adj: &HashMap<String, Vec<String>>) -> HashSet<String> {
    let mut visited = seeds.clone();
    let mut frontier = seeds.clone();

    for _ in 0..depth {
        let mut next_frontier = HashSet::new();
        for node_id in &frontier {
            if let Some(neighbors) = adj.get(node_id) {
                for neighbor in neighbors {
                    if !visited.contains(neighbor) {
                        visited.insert(neighbor.clone());
                        next_frontier.insert(neighbor.clone());
                    }
                }
            }
        }
        frontier = next_frontier;
    }
    visited
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() > max_chars {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{}...", truncated)
    } else {
        s.to_string()
    }
}

/// Truncate to a limited number of lines, each with a max character count
fn truncate_lines(s: &str, max_lines: usize, max_chars_per_line: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let truncated_lines: Vec<String> = lines
        .iter()
        .take(max_lines)
        .map(|line| {
            if line.chars().count() > max_chars_per_line {
                let truncated: String = line.chars().take(max_chars_per_line).collect();
                format!("{}...", truncated)
            } else {
                line.to_string()
            }
        })
        .collect();

    let result = truncated_lines.join("\n");
    if lines.len() > max_lines {
        format!("{}...", result)
    } else {
        result
    }
}

fn format_timestamp(ts: &str) -> String {
    use chrono::DateTime;

    if let Ok(parsed) = DateTime::parse_from_rfc3339(ts) {
        parsed.format("%b %d %H:%M").to_string()
    } else {
        ts.to_string()
    }
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod app_tests;
