//! Main application state and UI.

use crate::api::{ApiClient, ImportanceStats};
use crate::graph::types::{PartialSummaryData, RoleFilter};
use crate::graph::{ForceLayout, GraphState};
use crate::settings::Settings;
use eframe::egui::{self, Color32, Pos2, Stroke, Vec2};
use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

/// Time range options for filtering
#[derive(Debug, Clone, Copy, PartialEq)]
enum TimeRange {
    Hour1,
    Hour6,
    Hour24,
    Day3,
    Week1,
    Week2,
    Month1,
    Month3,
}

impl TimeRange {
    fn hours(&self) -> f32 {
        match self {
            TimeRange::Hour1 => 1.0,
            TimeRange::Hour6 => 6.0,
            TimeRange::Hour24 => 24.0,
            TimeRange::Day3 => 72.0,
            TimeRange::Week1 => 168.0,
            TimeRange::Week2 => 336.0,
            TimeRange::Month1 => 720.0,
            TimeRange::Month3 => 2160.0,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            TimeRange::Hour1 => "Past hour",
            TimeRange::Hour6 => "Past 6 hours",
            TimeRange::Hour24 => "Past 24 hours",
            TimeRange::Day3 => "Past 3 days",
            TimeRange::Week1 => "Past week",
            TimeRange::Week2 => "Past 2 weeks",
            TimeRange::Month1 => "Past month",
            TimeRange::Month3 => "Past 3 months",
        }
    }

    /// Find the TimeRange that matches the given hours value
    fn from_hours(hours: f32) -> Self {
        match hours as i32 {
            1 => TimeRange::Hour1,
            6 => TimeRange::Hour6,
            24 => TimeRange::Hour24,
            72 => TimeRange::Day3,
            168 => TimeRange::Week1,
            336 => TimeRange::Week2,
            720 => TimeRange::Month1,
            2160 => TimeRange::Month3,
            _ => TimeRange::Hour24, // Default fallback
        }
    }
}

/// Main dashboard application
pub struct DashboardApp {
    // Persistent settings
    settings: Settings,
    settings_dirty: bool,

    // API client
    api: ApiClient,
    api_connected: bool,
    api_error: Option<String>,

    // Graph state
    graph: GraphState,
    layout: ForceLayout,

    // UI state
    time_range: TimeRange,
    node_size: f32,
    show_arrows: bool,
    loading: bool,
    timeline_enabled: bool,

    // Recency scaling
    recency_min_scale: f32,
    recency_decay_rate: f32,

    // Importance filtering
    importance_threshold: f32,
    importance_filter_enabled: bool,

    // Role filtering (hide Claude/User messages)
    role_filter: RoleFilter,

    // Importance scoring (async backfill)
    importance_scoring: bool,
    importance_result: Option<String>,
    importance_receiver: Option<Receiver<Result<serde_json::Value, String>>>,
    importance_start_time: Option<Instant>,
    importance_initial_unscored: i64,
    importance_last_poll: Option<Instant>,
    importance_progress: Option<(i64, i64)>, // (scored, total)
    importance_since_days: f32, // Days filter for "Score Recent" button

    // Node sizing mode
    size_by_importance: bool,

    // Temporal edge opacity
    temporal_edge_opacity: f32,

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

    // Summary panel state
    summary_node_id: Option<String>,
    summary_session_id: Option<String>,
    summary_timestamp: Option<String>,
    summary_loading: bool,
    summary_data: Option<PartialSummaryData>,
    summary_error: Option<String>,
    summary_receiver: Option<Receiver<Result<PartialSummaryData, String>>>,

    // Double-click detection
    last_click_time: Instant,
    last_click_node: Option<String>,
}

impl DashboardApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        // Load persisted settings
        let settings = Settings::load();

        // Create layout with settings applied
        let mut layout = ForceLayout::default();
        layout.repulsion = settings.repulsion;
        layout.attraction = settings.attraction;
        layout.centering = settings.centering;
        layout.size_repulsion_weight = settings.size_repulsion_weight;
        layout.temporal_strength = settings.temporal_strength;

        // Create graph state with settings applied
        let mut graph = GraphState::new();
        graph.physics_enabled = settings.physics_enabled;
        graph.color_by_project = settings.color_by_project;
        graph.temporal_attraction_enabled = settings.temporal_attraction_enabled;
        graph.temporal_window_secs = settings.temporal_window_mins as f64 * 60.0;
        graph.timeline.speed = settings.timeline_speed;
        graph.timeline.spacing_mode = if settings.timeline_spacing_even {
            crate::graph::types::TimelineSpacingMode::EvenSpacing
        } else {
            crate::graph::types::TimelineSpacingMode::TimeBased
        };

        let mut app = Self {
            settings: settings.clone(),
            settings_dirty: false,

            api: ApiClient::new(),
            api_connected: false,
            api_error: None,
            graph,
            layout,
            time_range: TimeRange::from_hours(settings.time_range_hours),
            node_size: settings.node_size,
            show_arrows: settings.show_arrows,
            loading: false,
            timeline_enabled: settings.timeline_enabled,
            recency_min_scale: settings.recency_min_scale,
            recency_decay_rate: settings.recency_decay_rate,
            importance_threshold: settings.importance_threshold,
            importance_filter_enabled: settings.importance_filter_enabled,
            role_filter: settings.role_filter,
            importance_scoring: false,
            importance_result: None,
            importance_receiver: None,
            importance_start_time: None,
            importance_initial_unscored: 0,
            importance_last_poll: None,
            importance_progress: None,
            importance_since_days: 7.0,
            size_by_importance: settings.size_by_importance,
            temporal_edge_opacity: settings.temporal_edge_opacity,
            pan_offset: Vec2::ZERO,
            zoom: 1.0,
            dragging: false,
            drag_start: None,
            timeline_dragging: false,
            last_playback_time: Instant::now(),
            last_frame: Instant::now(),
            frame_times: Vec::with_capacity(60),
            fps: 0.0,

            // Summary panel state
            summary_node_id: None,
            summary_session_id: None,
            summary_timestamp: None,
            summary_loading: false,
            summary_data: None,
            summary_error: None,
            summary_receiver: None,

            // Double-click detection
            last_click_time: Instant::now(),
            last_click_node: None,
        };

        // Check API connection and load initial data
        app.check_api();
        if app.api_connected {
            app.load_graph();
        }

        app
    }

    fn check_api(&mut self) {
        match self.api.health() {
            Ok(true) => {
                self.api_connected = true;
                self.api_error = None;
            }
            Ok(false) => {
                self.api_connected = false;
                self.api_error = Some("API unhealthy".to_string());
            }
            Err(e) => {
                self.api_connected = false;
                self.api_error = Some(e);
            }
        }
    }

    fn load_graph(&mut self) {
        self.loading = true;

        match self.api.fetch_graph(self.time_range.hours(), None) {
            Ok(data) => {
                // Initialize with centered bounds
                let bounds = egui::Rect::from_center_size(
                    Pos2::new(400.0, 300.0),
                    Vec2::new(600.0, 400.0),
                );
                self.graph.load(data, bounds);
                self.loading = false;
            }
            Err(e) => {
                self.api_error = Some(e);
                self.loading = false;
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

    /// Sync current app state to settings and mark as needing save
    fn mark_settings_dirty(&mut self) {
        // Sync all current values to settings
        self.settings.time_range_hours = self.time_range.hours();
        self.settings.node_size = self.node_size;
        self.settings.show_arrows = self.show_arrows;
        self.settings.timeline_enabled = self.timeline_enabled;
        self.settings.size_by_importance = self.size_by_importance;
        self.settings.color_by_project = self.graph.color_by_project;
        self.settings.timeline_spacing_even = self.graph.timeline.spacing_mode == crate::graph::types::TimelineSpacingMode::EvenSpacing;
        self.settings.timeline_speed = self.graph.timeline.speed;
        self.settings.importance_threshold = self.importance_threshold;
        self.settings.importance_filter_enabled = self.importance_filter_enabled;
        self.settings.role_filter = self.role_filter;
        self.settings.physics_enabled = self.graph.physics_enabled;
        self.settings.repulsion = self.layout.repulsion;
        self.settings.attraction = self.layout.attraction;
        self.settings.centering = self.layout.centering;
        self.settings.size_repulsion_weight = self.layout.size_repulsion_weight;
        self.settings.temporal_strength = self.layout.temporal_strength;
        self.settings.temporal_attraction_enabled = self.graph.temporal_attraction_enabled;
        self.settings.temporal_window_mins = (self.graph.temporal_window_secs / 60.0) as f32;
        self.settings.temporal_edge_opacity = self.temporal_edge_opacity;
        self.settings.recency_min_scale = self.recency_min_scale;
        self.settings.recency_decay_rate = self.recency_decay_rate;

        self.settings_dirty = true;
    }

    /// Save settings if dirty
    fn save_settings_if_dirty(&mut self) {
        if self.settings_dirty {
            self.settings.save();
            self.settings_dirty = false;
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

    /// Trigger summary generation for a double-clicked node
    fn trigger_summary_for_node(&mut self, node_id: String) {
        if let Some(node) = self.graph.get_node(&node_id) {
            let session_id = node.session_id.clone();
            let timestamp = match &node.timestamp {
                Some(ts) => ts.clone(),
                None => {
                    self.summary_error = Some("Node has no timestamp".to_string());
                    return;
                }
            };

            // Store state
            self.summary_node_id = Some(node_id);
            self.summary_session_id = Some(session_id.clone());
            self.summary_timestamp = Some(timestamp.clone());
            self.summary_loading = true;
            self.summary_data = None;
            self.summary_error = None;

            // Create channel for async result
            let (tx, rx) = mpsc::channel();
            self.summary_receiver = Some(rx);

            // Spawn thread to fetch summary
            let api = ApiClient::new();
            std::thread::spawn(move || {
                let result = api.fetch_partial_summary(&session_id, &timestamp);
                let _ = tx.send(result);
            });
        }
    }

    fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.heading("Graph Controls");
        ui.add_space(10.0);

        // API status
        ui.horizontal(|ui| {
            if self.api_connected {
                ui.colored_label(Color32::GREEN, "● API Connected");
            } else {
                ui.colored_label(Color32::RED, "● API Disconnected");
                if ui.button("Retry").clicked() {
                    self.check_api();
                    if self.api_connected {
                        self.load_graph();
                    }
                }
            }
        });

        if let Some(ref err) = self.api_error {
            ui.colored_label(Color32::RED, format!("Error: {}", err));
        }

        ui.add_space(10.0);

        // Data Selection section
        egui::CollapsingHeader::new("Data Selection")
            .default_open(true)
            .show(ui, |ui| {
                let prev_range = self.time_range;
                egui::ComboBox::from_id_salt("time_range")
                    .selected_text(self.time_range.label())
                    .show_ui(ui, |ui| {
                        for range in [
                            TimeRange::Hour1,
                            TimeRange::Hour6,
                            TimeRange::Hour24,
                            TimeRange::Day3,
                            TimeRange::Week1,
                            TimeRange::Week2,
                            TimeRange::Month1,
                            TimeRange::Month3,
                        ] {
                            ui.selectable_value(&mut self.time_range, range, range.label());
                        }
                    });

                if self.time_range != prev_range {
                    self.mark_settings_dirty();
                    self.load_graph();
                }

                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    if ui.button("⟳ Reload").clicked() {
                        self.load_graph();
                    }
                    if ui.button("↺ Reset All").clicked() {
                        // Reset all UI state to defaults
                        let defaults = Settings::default();
                        self.time_range = TimeRange::from_hours(defaults.time_range_hours);
                        self.node_size = defaults.node_size;
                        self.show_arrows = defaults.show_arrows;
                        self.timeline_enabled = defaults.timeline_enabled;
                        self.size_by_importance = defaults.size_by_importance;
                        self.graph.color_by_project = defaults.color_by_project;
                        self.graph.physics_enabled = defaults.physics_enabled;
                        self.recency_min_scale = defaults.recency_min_scale;
                        self.recency_decay_rate = defaults.recency_decay_rate;
                        self.importance_threshold = defaults.importance_threshold;
                        self.importance_filter_enabled = defaults.importance_filter_enabled;
                        self.role_filter = defaults.role_filter;
                        self.layout.repulsion = defaults.repulsion;
                        self.layout.attraction = defaults.attraction;
                        self.layout.centering = defaults.centering;
                        self.layout.size_repulsion_weight = defaults.size_repulsion_weight;
                        self.layout.temporal_strength = defaults.temporal_strength;
                        self.graph.temporal_attraction_enabled = defaults.temporal_attraction_enabled;
                        self.graph.temporal_window_secs = defaults.temporal_window_mins as f64 * 60.0;
                        self.temporal_edge_opacity = defaults.temporal_edge_opacity;
                        self.graph.timeline.speed = defaults.timeline_speed;
                        self.graph.timeline.spacing_mode = if defaults.timeline_spacing_even {
                            crate::graph::types::TimelineSpacingMode::EvenSpacing
                        } else {
                            crate::graph::types::TimelineSpacingMode::TimeBased
                        };
                        self.pan_offset = Vec2::ZERO;
                        self.zoom = 1.0;
                        self.mark_settings_dirty();
                        self.load_graph();
                    }
                });
            });

        // Display section
        egui::CollapsingHeader::new("Display")
            .default_open(true)
            .show(ui, |ui| {
                let prev_node_size = self.node_size;
                ui.add(egui::Slider::new(&mut self.node_size, 5.0..=50.0).text("Node size"));
                if (self.node_size - prev_node_size).abs() > 0.01 {
                    self.mark_settings_dirty();
                }

                ui.add_space(5.0);
                ui.label("Size by:");
                let prev_size_by_importance = self.size_by_importance;
                ui.horizontal(|ui| {
                    if ui.selectable_label(!self.size_by_importance, "Recency").clicked() {
                        self.size_by_importance = false;
                    }
                    if ui.selectable_label(self.size_by_importance, "Importance").clicked() {
                        self.size_by_importance = true;
                    }
                });
                if self.size_by_importance != prev_size_by_importance {
                    self.mark_settings_dirty();
                }

                ui.add_space(5.0);
                let prev_show_arrows = self.show_arrows;
                let prev_timeline_enabled = self.timeline_enabled;
                ui.checkbox(&mut self.show_arrows, "Show arrows");
                ui.checkbox(&mut self.timeline_enabled, "Timeline scrubber");
                if self.show_arrows != prev_show_arrows || self.timeline_enabled != prev_timeline_enabled {
                    self.mark_settings_dirty();
                }

                // Spacing mode toggle (only show when timeline is enabled)
                if self.timeline_enabled {
                    ui.horizontal(|ui| {
                        ui.label("Spacing:");
                        let is_even = self.graph.timeline.spacing_mode == crate::graph::types::TimelineSpacingMode::EvenSpacing;
                        if ui.selectable_label(!is_even, "Time").clicked() {
                            self.graph.timeline.spacing_mode = crate::graph::types::TimelineSpacingMode::TimeBased;
                            self.graph.update_visible_nodes();
                            self.mark_settings_dirty();
                        }
                        if ui.selectable_label(is_even, "Even").clicked() {
                            self.graph.timeline.spacing_mode = crate::graph::types::TimelineSpacingMode::EvenSpacing;
                            self.graph.update_visible_nodes();
                            self.mark_settings_dirty();
                        }
                    });
                }

                ui.add_space(5.0);
                ui.label("Color by:");
                let prev_color_by_project = self.graph.color_by_project;
                ui.horizontal(|ui| {
                    if ui.selectable_label(self.graph.color_by_project, "Project").clicked() {
                        self.graph.color_by_project = true;
                    }
                    if ui.selectable_label(!self.graph.color_by_project, "Session").clicked() {
                        self.graph.color_by_project = false;
                    }
                });
                if self.graph.color_by_project != prev_color_by_project {
                    self.mark_settings_dirty();
                }
            });

        // Filtering section
        egui::CollapsingHeader::new("Filtering")
            .default_open(true)
            .show(ui, |ui| {
                let prev_filter_enabled = self.importance_filter_enabled;
                ui.checkbox(&mut self.importance_filter_enabled, "Filter by importance");
                if self.importance_filter_enabled != prev_filter_enabled {
                    self.mark_settings_dirty();
                }
                if self.importance_filter_enabled {
                    let prev_threshold = self.importance_threshold;
                    ui.add(egui::Slider::new(&mut self.importance_threshold, 0.0..=1.0)
                        .text("Min importance")
                        .fixed_decimals(2));
                    if (self.importance_threshold - prev_threshold).abs() > 0.001 {
                        self.mark_settings_dirty();
                    }
                    // Show count
                    let visible = self.graph.data.nodes.iter()
                        .filter(|n| n.importance_score.map_or(true, |s| s >= self.importance_threshold))
                        .count();
                    ui.label(format!("Showing: {} / {} nodes", visible, self.graph.data.nodes.len()));
                }

                ui.add_space(5.0);

                // Role filter toggle
                ui.label("Show messages:");
                let prev_role_filter = self.role_filter;
                ui.horizontal(|ui| {
                    if ui.selectable_label(self.role_filter == RoleFilter::ShowAll, "All").clicked() {
                        self.role_filter = RoleFilter::ShowAll;
                    }
                    if ui.selectable_label(self.role_filter == RoleFilter::HideClaude, "You only").clicked() {
                        self.role_filter = RoleFilter::HideClaude;
                    }
                    if ui.selectable_label(self.role_filter == RoleFilter::HideUser, "Claude only").clicked() {
                        self.role_filter = RoleFilter::HideUser;
                    }
                });
                if self.role_filter != prev_role_filter {
                    self.mark_settings_dirty();
                }

                ui.add_space(5.0);
                ui.separator();

                // Helper to start scoring with optional since_days filter
                let start_scoring = |app: &mut DashboardApp, since_days: Option<f32>| {
                    app.importance_scoring = true;
                    app.importance_result = None;
                    app.importance_start_time = Some(Instant::now());
                    app.importance_last_poll = Some(Instant::now());
                    app.importance_progress = None;

                    // Fetch initial stats to track progress
                    if let Ok(stats) = app.api.fetch_importance_stats() {
                        app.importance_initial_unscored = stats.unscored_messages;
                        app.importance_progress = Some((0, stats.unscored_messages));
                    }

                    // Create channel for async result
                    let (tx, rx) = mpsc::channel();
                    app.importance_receiver = Some(rx);

                    // Spawn thread to trigger backfill
                    let api = ApiClient::new();
                    std::thread::spawn(move || {
                        let result = api.trigger_importance_backfill(since_days);
                        let _ = tx.send(result);
                    });
                };

                // Score All button - scores all unprocessed messages
                ui.horizontal(|ui| {
                    if ui.add_enabled(!self.importance_scoring, egui::Button::new("Score All")).clicked() {
                        start_scoring(self, None);
                    }
                    if self.importance_scoring {
                        ui.spinner();
                        if ui.button("Cancel").clicked() {
                            self.importance_scoring = false;
                            self.importance_receiver = None;
                            self.importance_start_time = None;
                            self.importance_last_poll = None;
                            self.importance_progress = None;
                            self.importance_result = Some("Cancelled".to_string());
                        }
                    }
                });

                // Score Recent - days input + button
                ui.horizontal(|ui| {
                    ui.label("Past");
                    ui.add(egui::DragValue::new(&mut self.importance_since_days)
                        .range(1.0..=365.0)
                        .speed(0.5)
                        .suffix(" days"));
                    if ui.add_enabled(!self.importance_scoring, egui::Button::new("Score")).clicked() {
                        let days = self.importance_since_days;
                        start_scoring(self, Some(days));
                    }
                });

                // Show progress or result
                if self.importance_scoring {
                    if let Some((scored, total)) = self.importance_progress {
                        let pct = if total > 0 { (scored as f64 / total as f64 * 100.0) as i64 } else { 0 };
                        let remaining = total - scored;

                        // Calculate time estimate based on rate
                        let time_str = if let Some(start) = self.importance_start_time {
                            let elapsed = start.elapsed().as_secs_f64();
                            if scored > 0 && elapsed > 0.0 {
                                let rate = scored as f64 / elapsed;
                                let eta_secs = remaining as f64 / rate;
                                if eta_secs < 60.0 {
                                    format!("~{}s", eta_secs as i64)
                                } else {
                                    format!("~{}m", (eta_secs / 60.0) as i64)
                                }
                            } else {
                                "calculating...".to_string()
                            }
                        } else {
                            "".to_string()
                        };

                        ui.label(format!("{}/{} ({}%) {}", scored, total, pct, time_str));
                    }
                } else if let Some(ref result) = self.importance_result {
                    ui.label(result);
                }
            });

        // Advanced section (collapsed by default)
        egui::CollapsingHeader::new("Advanced")
            .default_open(false)
            .show(ui, |ui| {
                let prev_physics = self.graph.physics_enabled;
                ui.checkbox(&mut self.graph.physics_enabled, "Physics enabled");
                if self.graph.physics_enabled != prev_physics {
                    self.mark_settings_dirty();
                }
                ui.add_space(5.0);
                ui.label("Physics Tuning");

                let prev_repulsion = self.layout.repulsion;
                let prev_attraction = self.layout.attraction;
                let prev_centering = self.layout.centering;
                let prev_size_weight = self.layout.size_repulsion_weight;

                ui.add(egui::Slider::new(&mut self.layout.repulsion, 10.0..=100000.0).logarithmic(true).text("Repulsion"));
                ui.add(egui::Slider::new(&mut self.layout.attraction, 0.0001..=10.0).logarithmic(true).text("Attraction"));
                ui.add(egui::Slider::new(&mut self.layout.centering, 0.00001..=0.1).logarithmic(true).text("Centering"));
                ui.add(egui::Slider::new(&mut self.layout.size_repulsion_weight, 0.0..=1.0)
                    .text("Size→Repulsion")
                    .fixed_decimals(2));

                if (self.layout.repulsion - prev_repulsion).abs() > 1.0
                    || (self.layout.attraction - prev_attraction).abs() > 0.0001
                    || (self.layout.centering - prev_centering).abs() > 0.000001
                    || (self.layout.size_repulsion_weight - prev_size_weight).abs() > 0.001
                {
                    self.mark_settings_dirty();
                }

                ui.add_space(10.0);
                ui.label("Temporal Attraction");

                // Cache values before closure to avoid borrow issues
                let temporal_enabled = self.graph.temporal_attraction_enabled;
                let mut new_temporal_enabled = temporal_enabled;
                ui.checkbox(&mut new_temporal_enabled, "Enable temporal clustering");
                if new_temporal_enabled != temporal_enabled {
                    self.graph.set_temporal_attraction_enabled(new_temporal_enabled);
                    self.mark_settings_dirty();
                }

                if self.graph.temporal_attraction_enabled {
                    let prev_temporal_strength = self.layout.temporal_strength;
                    ui.add(egui::Slider::new(&mut self.layout.temporal_strength, 0.001..=2.0)
                        .logarithmic(true)
                        .text("Strength"));
                    if (self.layout.temporal_strength - prev_temporal_strength).abs() > 0.001 {
                        self.mark_settings_dirty();
                    }

                    // Temporal window slider (in minutes for UX, stored as seconds)
                    let mut window_mins = (self.graph.temporal_window_secs / 60.0) as f32;
                    let prev_window_mins = window_mins;
                    ui.add(egui::Slider::new(&mut window_mins, 1.0..=60.0)
                        .text("Window (min)")
                        .fixed_decimals(0));
                    if (window_mins - prev_window_mins).abs() > 0.1 {
                        self.graph.set_temporal_window(window_mins as f64 * 60.0);
                        self.mark_settings_dirty();
                    }

                    // Temporal edge opacity slider
                    let prev_opacity = self.temporal_edge_opacity;
                    ui.add(egui::Slider::new(&mut self.temporal_edge_opacity, 0.0..=1.0)
                        .text("Edge opacity")
                        .fixed_decimals(2));
                    if (self.temporal_edge_opacity - prev_opacity).abs() > 0.001 {
                        self.mark_settings_dirty();
                    }

                    // Show temporal edge count
                    let temporal_count = self.graph.data.edges.iter().filter(|e| e.is_temporal).count();
                    ui.label(format!("Temporal edges: {}", temporal_count));
                }

                ui.add_space(10.0);
                ui.label("Recency Scaling");
                let prev_min_scale = self.recency_min_scale;
                let prev_decay_rate = self.recency_decay_rate;
                ui.add(egui::Slider::new(&mut self.recency_min_scale, 0.001..=1.0).logarithmic(true).text("Min scale"));
                ui.add(egui::Slider::new(&mut self.recency_decay_rate, 0.1..=100.0).logarithmic(true).text("Decay rate"));
                if (self.recency_min_scale - prev_min_scale).abs() > 0.0001
                    || (self.recency_decay_rate - prev_decay_rate).abs() > 0.01
                {
                    self.mark_settings_dirty();
                }
            });

        ui.add_space(10.0);
        ui.separator();

        // Info section (always visible)
        ui.label("Info");
        let visible_count = self.graph.timeline.visible_nodes.len();
        let total_count = self.graph.data.nodes.len();
        if self.timeline_enabled && visible_count < total_count {
            ui.label(format!("Nodes: {} / {}", visible_count, total_count));
        } else {
            ui.label(format!("Nodes: {}", total_count));
        }
        ui.label(format!("Edges: {}", self.graph.data.edges.len()));
        ui.label(format!("FPS: {:.1}", self.fps));

        let user_count = self.graph.data.nodes.iter().filter(|n| n.role == crate::graph::types::Role::User).count();
        let assistant_count = self.graph.data.nodes.iter().filter(|n| n.role == crate::graph::types::Role::Assistant).count();
        ui.label(format!("You: {} | Claude: {}", user_count, assistant_count));

        ui.add_space(5.0);
        ui.separator();

        // Zoom controls
        ui.label("View");
        ui.horizontal(|ui| {
            if ui.button("Reset View").clicked() {
                self.pan_offset = Vec2::ZERO;
                self.zoom = 1.0;
            }
            ui.label(format!("Zoom: {:.0}%", self.zoom * 100.0));
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
                // Show just the time portion
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

        ui.add_space(10.0);
        ui.separator();

        // Point-in-Time Summary Panel
        ui.collapsing("Point-in-Time Summary", |ui| {
            if self.summary_loading {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Generating summary...");
                });
            } else if let Some(ref error) = self.summary_error {
                ui.colored_label(Color32::RED, format!("Error: {}", error));
                if ui.button("Dismiss").clicked() {
                    self.summary_error = None;
                    self.summary_node_id = None;
                }
            } else if let Some(ref data) = self.summary_data {
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
                    .id_salt("summary_scroll")
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
                    ui.label(egui::RichText::new("Completed").strong().color(Color32::GREEN));
                    egui::ScrollArea::vertical()
                        .max_height(60.0)
                        .id_salt("completed_scroll")
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
                        .id_salt("failed_scroll")
                        .show(ui, |ui| {
                            for line in data.unsuccessful_attempts.split('\n').filter(|l| !l.is_empty()) {
                                ui.label(line);
                            }
                        });
                }

                ui.add_space(5.0);
                if ui.button("Clear").clicked() {
                    self.summary_data = None;
                    self.summary_node_id = None;
                }
            } else {
                ui.label("Double-click a node to generate");
                ui.label("a summary up to that point.");
            }
        });

        ui.add_space(10.0);

        // Legend
        ui.separator();
        if self.graph.color_by_project {
            ui.label("Projects");
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

        // Build combined visibility set for physics (timeline + importance + role filtering)
        let role_filter = self.role_filter;
        let physics_visible: Option<HashSet<String>> = if self.timeline_enabled || self.importance_filter_enabled || role_filter != RoleFilter::ShowAll {
            let visible: HashSet<String> = self.graph.data.nodes.iter()
                .filter(|node| {
                    // Check timeline visibility
                    if self.timeline_enabled && !self.graph.timeline.visible_nodes.contains(&node.id) {
                        return false;
                    }
                    // Check importance visibility
                    if self.importance_filter_enabled {
                        if let Some(score) = node.importance_score {
                            if score < self.importance_threshold {
                                return false;
                            }
                        }
                    }
                    // Check role visibility
                    if !role_filter.is_visible(&node.role) {
                        return false;
                    }
                    true
                })
                .map(|n| n.id.clone())
                .collect();
            Some(visible)
        } else {
            None // No filtering - physics uses all nodes
        };

        // Run physics simulation (uses graph-space center, unaffected by viewport pan)
        self.layout.step(&mut self.graph, center, physics_visible.as_ref());

        // Cache values for transform closure to avoid borrowing self
        let pan_offset = self.pan_offset;
        let zoom = self.zoom;

        // Transform helper: graph space -> screen space
        // Pan is in screen space (applied after zoom) for 1:1 movement at any zoom level
        let transform = |pos: Pos2| -> Pos2 {
            let centered = pos.to_vec2() - center.to_vec2();
            center + centered * zoom + pan_offset
        };

        // Draw edges first (behind nodes)
        // When filtering (importance or role), we need to draw "bridge" edges that skip filtered nodes
        // to keep session flows connected.
        let mut drawn_bridges: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
        let importance_threshold = self.importance_threshold;
        let importance_filter_enabled = self.importance_filter_enabled;

        for edge in &self.graph.data.edges {
            // Skip edges for hidden nodes when timeline is enabled
            if self.timeline_enabled && !self.graph.is_edge_visible(edge) {
                continue;
            }

            // Helper: check if a node is filtered out (by importance or role)
            let is_node_filtered = |node_id: &str| -> bool {
                if let Some(node) = self.graph.get_node(node_id) {
                    // Check importance filter
                    if importance_filter_enabled {
                        if let Some(score) = node.importance_score {
                            if score < importance_threshold {
                                return true;
                            }
                        }
                    }
                    // Check role filter
                    if !role_filter.is_visible(&node.role) {
                        return true;
                    }
                    false
                } else {
                    true // Node not found = filtered
                }
            };

            // Determine source and target for this edge (with bridging if needed)
            let needs_bridging = importance_filter_enabled || role_filter != RoleFilter::ShowAll;
            let (draw_source, draw_target, is_bridge) = if needs_bridging {
                let is_session_edge = !edge.is_temporal && !edge.is_similarity && !edge.is_topic && !edge.is_obsidian;

                let source_filtered = is_node_filtered(&edge.source);
                let target_filtered = is_node_filtered(&edge.target);

                if is_session_edge && source_filtered {
                    // Source is filtered - skip (its predecessor will bridge to target or beyond)
                    continue;
                } else if is_session_edge && target_filtered {
                    // Target is filtered - find next visible node in chain to bridge to
                    if let Some(next_visible) = self.graph.next_visible_in_chain(&edge.source, importance_threshold, role_filter) {
                        // Check if we already drew this bridge
                        let bridge_key = (edge.source.clone(), next_visible.clone());
                        if drawn_bridges.contains(&bridge_key) {
                            continue;
                        }
                        drawn_bridges.insert(bridge_key);
                        (edge.source.clone(), next_visible, true)
                    } else {
                        continue; // No visible target to bridge to
                    }
                } else if source_filtered || target_filtered {
                    // Non-session edge with filtered endpoint - skip entirely
                    continue;
                } else {
                    // Both visible - draw normally
                    (edge.source.clone(), edge.target.clone(), false)
                }
            } else {
                // No filtering - draw all edges
                (edge.source.clone(), edge.target.clone(), false)
            };

            let source_pos = match self.graph.get_pos(&draw_source) {
                Some(p) => transform(p),
                None => continue,
            };
            let target_pos = match self.graph.get_pos(&draw_target) {
                Some(p) => transform(p),
                None => continue,
            };

            // Use dashed style for bridge edges (optional visual hint)
            // Temporal edges use configurable opacity
            let base_opacity = if edge.is_temporal {
                self.temporal_edge_opacity
            } else if is_bridge {
                0.3
            } else {
                0.5
            };
            let color = self.graph.edge_color(edge).gamma_multiply(base_opacity);
            let stroke = Stroke::new(1.5 * self.zoom, color);

            painter.line_segment([source_pos, target_pos], stroke);

            // Draw arrow if enabled
            if self.show_arrows {
                let delta = target_pos - source_pos;
                let length = delta.length();
                // Skip arrow if nodes are at same position (prevents NaN from normalized())
                if length > 0.01 {
                    let dir = delta / length;
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
        }

        // Detect hover - always select closest visible node to cursor
        let mut new_hovered = None;
        if let Some(hover_pos) = response.hover_pos() {
            let mut closest: Option<(String, f32)> = None; // (node_id, distance)

            for node in &self.graph.data.nodes {
                // Skip hidden nodes when timeline is enabled
                if self.timeline_enabled && !self.graph.is_node_visible(&node.id) {
                    continue;
                }
                // Skip nodes below importance threshold when filter is enabled
                if self.importance_filter_enabled {
                    if let Some(score) = node.importance_score {
                        if score < self.importance_threshold {
                            continue;
                        }
                    }
                }
                // Skip nodes hidden by role filter
                if !role_filter.is_visible(&node.role) {
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
        self.graph.hovered_node = new_hovered;

        // Draw nodes (only visible ones when filtering is enabled)
        for node in &self.graph.data.nodes {
            // Skip hidden nodes when timeline is enabled
            if self.timeline_enabled && !self.graph.is_node_visible(&node.id) {
                continue;
            }

            // Skip nodes below importance threshold when filter is enabled
            if self.importance_filter_enabled {
                if let Some(score) = node.importance_score {
                    if score < self.importance_threshold {
                        continue;
                    }
                }
            }

            // Skip nodes hidden by role filter
            if !role_filter.is_visible(&node.role) {
                continue;
            }

            if let Some(pos) = self.graph.get_pos(&node.id) {
                let screen_pos = transform(pos);
                let is_hovered = self.graph.hovered_node.as_ref() == Some(&node.id);
                let is_selected = self.graph.selected_node.as_ref() == Some(&node.id);

                // Node size scaling based on mode (recency or importance)
                let min_s = self.recency_min_scale;
                let size_scale = if self.size_by_importance {
                    // Importance-based: scale by importance score (0.0-1.0)
                    node.importance_score
                        .map(|score| min_s + (1.0 - min_s) * score)
                        .unwrap_or(0.5) // No score = medium size
                } else {
                    // Recency-based: exponential decay from scrubber position
                    let decay = self.recency_decay_rate;
                    if self.graph.timeline.max_time > self.graph.timeline.min_time {
                        if let Some(node_time) = node.timestamp_secs() {
                            let time_range = self.graph.timeline.max_time - self.graph.timeline.min_time;
                            let scrubber_time = self.graph.timeline.time_at_position(self.graph.timeline.position);
                            let distance = (scrubber_time - node_time).abs();
                            let normalized_distance = (distance / time_range).clamp(0.0, 1.0);
                            min_s + (1.0 - min_s) * (-decay * normalized_distance as f32).exp()
                        } else {
                            0.5 // No timestamp = medium size
                        }
                    } else {
                        1.0 // No time range = all same size
                    }
                };

                let base_size = self.node_size * self.zoom * size_scale;
                let size = if is_hovered || is_selected {
                    base_size * 1.3
                } else {
                    base_size
                };

                // Use project or session color based on mode
                let color = self.graph.node_color(node);

                // Draw node
                painter.circle_filled(screen_pos, size, color);

                // Draw inner black circle for Claude responses
                if node.role == crate::graph::types::Role::Assistant {
                    let inner_size = size * 0.4;
                    painter.circle_filled(screen_pos, inner_size, Color32::BLACK);
                }

                // Draw border - cyan for summary node, yellow for selected, white for hovered
                let is_summary_node = self.summary_node_id.as_ref() == Some(&node.id);
                let border_color = if is_summary_node {
                    Color32::from_rgb(0, 255, 255) // Cyan for summary node
                } else if is_selected {
                    Color32::YELLOW
                } else if is_hovered {
                    Color32::WHITE
                } else {
                    color.gamma_multiply(0.7)
                };
                let border_width = if is_summary_node { 4.0 } else { 2.0 };
                painter.circle_stroke(screen_pos, size, Stroke::new(border_width, border_color));
            }
        }

        // Handle click selection with double-click detection
        // Use the already-computed closest node from hover detection
        if response.clicked() {
            let clicked_node = self.graph.hovered_node.clone();

            // Check for double-click (same node within 500ms)
            if let Some(ref node_id) = clicked_node {
                let now = Instant::now();
                let elapsed = now.duration_since(self.last_click_time).as_millis();
                let same_node = self.last_click_node.as_ref() == Some(node_id);
                let is_double_click = same_node && elapsed < 500;

                if is_double_click {
                    self.trigger_summary_for_node(node_id.clone());
                }

                self.last_click_time = now;
                self.last_click_node = clicked_node.clone();
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

                    // Draw tooltip background
                    let importance_str = match node.importance_score {
                        Some(score) => format!("{:.0}%", score * 100.0),
                        None => "—".to_string(),
                    };
                    let tooltip_text = format!(
                        "{}\n{}\n\nSession: {}\nProject: {}\nImportance: {}",
                        node.role.label(),
                        truncate(&node.content_preview, 80),
                        node.session_short,
                        node.project,
                        importance_str
                    );

                    let galley = painter.layout_no_wrap(
                        tooltip_text,
                        egui::FontId::default(),
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

        // Loading indicator
        if self.loading {
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                "Loading...",
                egui::FontId::proportional(24.0),
                Color32::WHITE,
            );
        }
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
        let visible_count = self.graph.timeline.visible_nodes.len();
        let total_count = self.graph.data.nodes.len();
        let start_time = self.graph.timeline.time_at_position(start_pos);
        let end_time = self.graph.timeline.time_at_position(end_pos);
        let start_time_str = self.graph.timeline.format_time(start_time);
        let end_time_str = self.graph.timeline.format_time(end_time);
        let timestamps: Vec<f64> = self.graph.timeline.timestamps.clone();
        let min_time = self.graph.timeline.min_time;
        let max_time = self.graph.timeline.max_time;
        let spacing_mode = self.graph.timeline.spacing_mode;
        let node_count = timestamps.len();

        // Helper to calculate position from time (time-based mode)
        let position_at_time = |t: f64| -> f32 {
            if max_time <= min_time {
                1.0
            } else {
                ((t - min_time) / (max_time - min_time)) as f32
            }
        };

        // Helper to calculate position for a node at index (respects spacing mode)
        let position_for_notch = |index: usize| -> f32 {
            match spacing_mode {
                crate::graph::types::TimelineSpacingMode::TimeBased => {
                    if index < node_count {
                        position_at_time(timestamps[index])
                    } else {
                        1.0
                    }
                }
                crate::graph::types::TimelineSpacingMode::EvenSpacing => {
                    if node_count <= 1 {
                        1.0
                    } else {
                        index as f32 / (node_count - 1) as f32
                    }
                }
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
            }

            if ui.button("⏭").clicked() {
                self.graph.timeline.position = 1.0;
                self.graph.update_visible_nodes();
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
                    self.mark_settings_dirty();
                }
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
            Color32::from_rgb(30, 33, 40)
        );

        // Draw notches for each node (positioned by spacing mode)
        let notch_color = Color32::from_rgb(60, 65, 75);
        for i in 0..node_count {
            let pos = position_for_notch(i);
            let x = rect.left() + pos * rect.width();
            painter.line_segment(
                [Pos2::new(x, rect.top() + 5.0), Pos2::new(x, rect.bottom() - 5.0)],
                Stroke::new(1.0, notch_color)
            );
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
            Color32::from_rgba_unmultiplied(255, 149, 0, 80)
        );

        // Draw start handle
        let handle_width = 8.0;
        let start_handle_rect = egui::Rect::from_center_size(
            Pos2::new(start_x, rect.center().y),
            Vec2::new(handle_width, rect.height() - 4.0)
        );
        painter.rect_filled(start_handle_rect, 2.0, Color32::from_rgb(100, 100, 120));

        // Draw end/position handle (main scrubber)
        let end_handle_rect = egui::Rect::from_center_size(
            Pos2::new(end_x, rect.center().y),
            Vec2::new(handle_width, rect.height() - 4.0)
        );
        painter.rect_filled(end_handle_rect, 2.0, Color32::from_rgb(255, 149, 0));

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
                    // Snap to nearest notch for smooth scrubbing (respects spacing mode)
                    let snapped = self.graph.timeline.snap_to_notch_modal(new_pos);
                    self.graph.timeline.position = snapped.max(self.graph.timeline.start_position + 0.01);
                }

                self.graph.update_visible_nodes();
                self.timeline_dragging = true;
            }
        } else {
            self.timeline_dragging = false;
        }

        // Handle click to jump
        if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let new_pos = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                let snapped = self.graph.timeline.snap_to_notch_modal(new_pos);
                self.graph.timeline.position = snapped.max(self.graph.timeline.start_position + 0.01);
                self.graph.update_visible_nodes();
            }
        }
    }
}

impl eframe::App for DashboardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_fps();

        // Check for summary result from background thread
        if let Some(ref rx) = self.summary_receiver {
            match rx.try_recv() {
                Ok(Ok(data)) => {
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

        // Poll importance stats while scoring is in progress
        if self.importance_scoring {
            let should_poll = self.importance_last_poll
                .map(|last| last.elapsed().as_secs_f64() >= 1.0)
                .unwrap_or(true);

            if should_poll {
                if let Ok(stats) = self.api.fetch_importance_stats() {
                    let scored = self.importance_initial_unscored - stats.unscored_messages;
                    self.importance_progress = Some((scored, self.importance_initial_unscored));
                    self.importance_last_poll = Some(Instant::now());
                }
            }
        }

        // Check for importance backfill result from background thread
        if let Some(ref rx) = self.importance_receiver {
            match rx.try_recv() {
                Ok(Ok(data)) => {
                    // Format the result nicely
                    let scored = data.get("messages_scored").and_then(|v| v.as_i64()).unwrap_or(0);
                    let sessions = data.get("sessions_processed").and_then(|v| v.as_i64()).unwrap_or(0);
                    self.importance_result = Some(format!("Scored {} msgs in {} sessions", scored, sessions));
                    self.importance_scoring = false;
                    self.importance_receiver = None;
                    self.importance_start_time = None;
                    self.importance_last_poll = None;
                    self.importance_progress = None;
                    // Reload graph to show new scores
                    self.load_graph();
                }
                Ok(Err(e)) => {
                    self.importance_result = Some(format!("Error: {}", e));
                    self.importance_scoring = false;
                    self.importance_receiver = None;
                    self.importance_start_time = None;
                    self.importance_last_poll = None;
                    self.importance_progress = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // Still loading, request repaint to check again
                    ctx.request_repaint();
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.importance_result = Some("Scoring cancelled".to_string());
                    self.importance_scoring = false;
                    self.importance_receiver = None;
                    self.importance_start_time = None;
                    self.importance_last_poll = None;
                    self.importance_progress = None;
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

            if self.graph.timeline.position >= 1.0 {
                self.graph.timeline.playing = false;
            }
        }

        // Request continuous repaint for physics simulation or playback
        if (self.graph.physics_enabled && !self.layout.is_settled(&self.graph))
            || self.graph.timeline.playing
        {
            ctx.request_repaint();
        }

        // Dark theme
        ctx.set_visuals(egui::Visuals::dark());

        // Sidebar
        egui::SidePanel::left("sidebar")
            .min_width(220.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.render_sidebar(ui);
                });
            });

        // Bottom timeline panel (only when enabled)
        if self.timeline_enabled {
            egui::TopBottomPanel::bottom("timeline")
                .min_height(80.0)
                .frame(egui::Frame::none()
                    .fill(Color32::from_rgb(20, 22, 28))
                    .inner_margin(egui::Margin::symmetric(12.0, 8.0)))
                .show(ctx, |ui| {
                    self.render_timeline(ui);
                });
        }

        // Main graph area
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::from_rgb(14, 17, 23)))
            .show(ctx, |ui| {
                self.render_graph(ui);
            });

        // Save settings if dirty (debounced by only saving when actually modified)
        self.save_settings_if_dirty();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Ensure settings are saved on exit
        self.mark_settings_dirty();
        self.save_settings_if_dirty();
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() > max_chars {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{}...", truncated)
    } else {
        s.to_string()
    }
}
