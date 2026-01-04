//! Main application state and UI.

use crate::api::{ApiClient, ImportanceStats};
use crate::graph::types::{PartialSummaryData, SessionSummaryData};
use crate::graph::{ForceLayout, GraphState};
use crate::settings::Settings;
use eframe::egui::{self, Color32, Pos2, Stroke, Vec2};
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

    fn from_hours(hours: f32) -> Self {
        if hours <= 1.0 {
            TimeRange::Hour1
        } else if hours <= 6.0 {
            TimeRange::Hour6
        } else if hours <= 24.0 {
            TimeRange::Hour24
        } else if hours <= 72.0 {
            TimeRange::Day3
        } else if hours <= 168.0 {
            TimeRange::Week1
        } else if hours <= 336.0 {
            TimeRange::Week2
        } else if hours <= 720.0 {
            TimeRange::Month1
        } else {
            TimeRange::Month3
        }
    }
}

/// Main dashboard application
pub struct DashboardApp {
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

    // Temporal edge opacity
    temporal_edge_opacity: f32,

    // Importance filtering
    importance_threshold: f32,
    importance_filter_enabled: bool,
    size_by_importance: bool,
    importance_stats: Option<ImportanceStats>,

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

    // Double-click detection
    last_click_time: Instant,
    last_click_node: Option<String>,

    // Settings persistence
    settings: Settings,
    settings_dirty: bool,
    last_settings_save: Instant,
}

impl DashboardApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        // Load saved settings
        let settings = Settings::load();

        // Create layout with saved physics settings
        let mut layout = ForceLayout::default();
        layout.repulsion = settings.repulsion;
        layout.attraction = settings.attraction;
        layout.centering = settings.centering;
        layout.temporal_strength = settings.temporal_strength;

        // Create graph state with saved settings
        let mut graph = GraphState::new();
        graph.physics_enabled = settings.physics_enabled;
        graph.color_by_project = settings.color_by_project;
        graph.temporal_attraction_enabled = settings.temporal_attraction_enabled;
        graph.temporal_window_secs = settings.temporal_window_mins as f64 * 60.0;
        graph.max_temporal_edges = settings.max_temporal_edges;

        let mut app = Self {
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
            temporal_edge_opacity: settings.temporal_edge_opacity,
            importance_threshold: settings.importance_threshold,
            importance_filter_enabled: settings.importance_filter_enabled,
            size_by_importance: settings.size_by_importance,
            importance_stats: None,
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

            // Double-click detection
            last_click_time: Instant::now(),
            last_click_node: None,

            // Settings persistence
            settings,
            settings_dirty: false,
            last_settings_save: Instant::now(),
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

    /// Mark settings as needing to be saved
    fn mark_settings_dirty(&mut self) {
        self.settings_dirty = true;
    }

    /// Copy current UI state to settings struct
    fn sync_settings_from_ui(&mut self) {
        self.settings.time_range_hours = self.time_range.hours();
        self.settings.node_size = self.node_size;
        self.settings.show_arrows = self.show_arrows;
        self.settings.timeline_enabled = self.timeline_enabled;
        self.settings.color_by_project = self.graph.color_by_project;
        self.settings.importance_threshold = self.importance_threshold;
        self.settings.importance_filter_enabled = self.importance_filter_enabled;
        self.settings.size_by_importance = self.size_by_importance;
        self.settings.physics_enabled = self.graph.physics_enabled;
        self.settings.repulsion = self.layout.repulsion;
        self.settings.attraction = self.layout.attraction;
        self.settings.centering = self.layout.centering;
        self.settings.temporal_strength = self.layout.temporal_strength;
        self.settings.temporal_attraction_enabled = self.graph.temporal_attraction_enabled;
        self.settings.temporal_window_mins = (self.graph.temporal_window_secs / 60.0) as f32;
        self.settings.temporal_edge_opacity = self.temporal_edge_opacity;
        self.settings.max_temporal_edges = self.graph.max_temporal_edges;
        self.settings.recency_min_scale = self.recency_min_scale;
        self.settings.recency_decay_rate = self.recency_decay_rate;
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

                // Fetch importance stats
                if let Ok(stats) = self.api.fetch_importance_stats() {
                    self.importance_stats = Some(stats);
                }
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

            // Also reset session summary state
            self.session_summary_data = None;
            self.session_summary_loading = true;

            // Create channels for async results
            let (tx, rx) = mpsc::channel();
            self.summary_receiver = Some(rx);

            let (session_tx, session_rx) = mpsc::channel();
            self.session_summary_receiver = Some(session_rx);

            // Spawn thread to fetch point-in-time summary
            let api = ApiClient::new();
            let session_id_for_partial = session_id.clone();
            std::thread::spawn(move || {
                let result = api.fetch_partial_summary(&session_id_for_partial, &timestamp);
                let _ = tx.send(result);
            });

            // Spawn thread to fetch full session summary (generate if missing)
            let api2 = ApiClient::new();
            std::thread::spawn(move || {
                let result = api2.fetch_session_summary(&session_id, true);
                let _ = session_tx.send(result);
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
                    self.load_graph();
                    self.mark_settings_dirty();
                }

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
                        self.recency_min_scale = 0.01;
                        self.recency_decay_rate = 3.0;
                        self.layout.repulsion = 10000.0;
                        self.layout.attraction = 0.1;
                        self.layout.centering = 0.0001;
                        self.pan_offset = Vec2::ZERO;
                        self.zoom = 1.0;
                        self.load_graph();
                    }
                });
            });

        // Display section
        egui::CollapsingHeader::new("Display")
            .default_open(true)
            .show(ui, |ui| {
                if ui.add(egui::Slider::new(&mut self.node_size, 5.0..=50.0).text("Node size")).changed() {
                    self.mark_settings_dirty();
                }
                if ui.checkbox(&mut self.size_by_importance, "Size by importance").changed() {
                    self.mark_settings_dirty();
                }
                if ui.checkbox(&mut self.show_arrows, "Show arrows").changed() {
                    self.mark_settings_dirty();
                }
                if ui.checkbox(&mut self.timeline_enabled, "Timeline scrubber").changed() {
                    self.mark_settings_dirty();
                }

                ui.add_space(5.0);
                ui.label("Color by:");
                ui.horizontal(|ui| {
                    if ui.selectable_label(self.graph.color_by_project, "Project").clicked() {
                        self.graph.color_by_project = true;
                        self.mark_settings_dirty();
                    }
                    if ui.selectable_label(!self.graph.color_by_project, "Session").clicked() {
                        self.graph.color_by_project = false;
                        self.mark_settings_dirty();
                    }
                });
            });

        // Filtering section
        egui::CollapsingHeader::new("Filtering")
            .default_open(true)
            .show(ui, |ui| {
                if ui.checkbox(&mut self.importance_filter_enabled, "Filter by importance").changed() {
                    self.mark_settings_dirty();
                }
                if self.importance_filter_enabled {
                    if ui.add(egui::Slider::new(&mut self.importance_threshold, 0.0..=1.0)
                        .text("Min importance")
                        .fixed_decimals(2)).changed() {
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
            });

        // Advanced section (collapsed by default)
        egui::CollapsingHeader::new("Advanced")
            .default_open(false)
            .show(ui, |ui| {
                if ui.checkbox(&mut self.graph.physics_enabled, "Physics enabled").changed() {
                    self.mark_settings_dirty();
                }
                ui.add_space(5.0);
                ui.label("Physics Tuning");
                if ui.add(egui::Slider::new(&mut self.layout.repulsion, 10.0..=100000.0).logarithmic(true).text("Repulsion")).changed() {
                    self.mark_settings_dirty();
                }
                if ui.add(egui::Slider::new(&mut self.layout.attraction, 0.0001..=10.0).logarithmic(true).text("Attraction")).changed() {
                    self.mark_settings_dirty();
                }
                if ui.add(egui::Slider::new(&mut self.layout.centering, 0.00001..=0.1).logarithmic(true).text("Centering")).changed() {
                    self.mark_settings_dirty();
                }

                ui.add_space(10.0);
                ui.label("Recency Scaling");
                if ui.add(egui::Slider::new(&mut self.recency_min_scale, 0.001..=1.0).logarithmic(true).text("Min scale")).changed() {
                    self.mark_settings_dirty();
                }
                if ui.add(egui::Slider::new(&mut self.recency_decay_rate, 0.1..=100.0).logarithmic(true).text("Decay rate")).changed() {
                    self.mark_settings_dirty();
                }

                ui.add_space(10.0);
                ui.separator();
                ui.label("Temporal Clustering");

                let temporal_enabled = self.graph.temporal_attraction_enabled;
                let mut new_temporal_enabled = temporal_enabled;
                ui.checkbox(&mut new_temporal_enabled, "Enable temporal edges");
                if new_temporal_enabled != temporal_enabled {
                    self.graph.set_temporal_attraction_enabled(new_temporal_enabled);
                    self.mark_settings_dirty();
                }

                if self.graph.temporal_attraction_enabled {
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
                        self.graph.set_temporal_window(window_mins as f64 * 60.0);
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
                                        self.graph.set_max_temporal_edges(value);
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
                    self.session_summary_data = None;
                }
            } else {
                ui.label("Double-click a node to generate");
                ui.label("a summary up to that point.");
            }
        });

        // Session Summary Panel (full session)
        ui.collapsing("Session Summary", |ui| {
            if self.session_summary_loading {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Generating summary...");
                });
            } else if let Some(ref data) = self.session_summary_data {
                // Check for errors first
                if let Some(ref err) = data.error {
                    ui.colored_label(Color32::RED, format!("Error: {}", err));
                } else if !data.exists {
                    ui.label("No summary could be generated.");
                } else {
                    // Show "just generated" badge if applicable
                    if data.generated {
                        ui.colored_label(Color32::from_rgb(34, 197, 94), "✓ Just generated");
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
                            .id_salt("session_summary_scroll")
                            .show(ui, |ui| {
                                ui.label(summary);
                            });
                        ui.add_space(5.0);
                    }

                    // Completed work
                    if let Some(ref completed) = data.completed_work {
                        if !completed.is_empty() {
                            ui.label(egui::RichText::new("Completed Work").strong().color(Color32::GREEN));
                            egui::ScrollArea::vertical()
                                .max_height(80.0)
                                .id_salt("session_completed_scroll")
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
                                .id_salt("session_requests_scroll")
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

        // Run physics simulation (uses graph-space center, unaffected by viewport pan)
        self.layout.step(&mut self.graph, center);

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
        for edge in &self.graph.data.edges {
            // Skip edges for hidden nodes when timeline is enabled
            if self.timeline_enabled && !self.graph.is_edge_visible(edge) {
                continue;
            }

            // Skip edges where either endpoint is below importance threshold
            if self.importance_filter_enabled {
                let source_below = self.graph.get_node(&edge.source)
                    .and_then(|n| n.importance_score)
                    .map_or(false, |s| s < self.importance_threshold);
                let target_below = self.graph.get_node(&edge.target)
                    .and_then(|n| n.importance_score)
                    .map_or(false, |s| s < self.importance_threshold);
                if source_below || target_below {
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

            let base_opacity = if edge.is_temporal {
                self.temporal_edge_opacity
            } else {
                0.5
            };
            let color = self.graph.edge_color(edge).gamma_multiply(base_opacity);
            let stroke = Stroke::new(1.5 * self.zoom, color);

            painter.line_segment([source_pos, target_pos], stroke);

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

        // Draw nodes (only visible ones when timeline is enabled)
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

            if let Some(pos) = self.graph.get_pos(&node.id) {
                let screen_pos = transform(pos);
                let is_hovered = self.graph.hovered_node.as_ref() == Some(&node.id);
                let is_selected = self.graph.selected_node.as_ref() == Some(&node.id);

                // Exponential scaling based on distance from scrubber position
                let min_s = self.recency_min_scale;
                let decay = self.recency_decay_rate;
                let recency_scale = if self.graph.timeline.max_time > self.graph.timeline.min_time {
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
                };

                // Importance scaling: scale node radius by importance score
                let importance_scale = if self.size_by_importance {
                    // Use importance score (0.3 to 1.0 range to keep minimum visible)
                    node.importance_score.unwrap_or(0.5) * 0.7 + 0.3
                } else {
                    1.0
                };

                let base_size = self.node_size * self.zoom * recency_scale * importance_scale;
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

                    // Build token info for assistant nodes
                    let token_str = if node.role == crate::graph::types::Role::Assistant {
                        let mut parts = Vec::new();
                        if let Some(output) = node.output_tokens {
                            parts.push(format!("out: {}", output));
                        }
                        if let Some(input) = node.input_tokens {
                            parts.push(format!("in: {}", input));
                        }
                        if let Some(cache) = node.cache_read_tokens {
                            if cache > 0 {
                                parts.push(format!("cache: {}", cache));
                            }
                        }
                        if parts.is_empty() {
                            String::new()
                        } else {
                            format!("\nTokens: {}", parts.join(", "))
                        }
                    } else {
                        String::new()
                    };

                    let tooltip_text = format!(
                        "{}\n{}\n\nSession: {}\nProject: {}\nImportance: {}{}",
                        node.role.label(),
                        truncate(&node.content_preview, 80),
                        node.session_short,
                        node.project,
                        importance_str,
                        token_str
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

        // Draw notches for each node timestamp
        let notch_color = Color32::from_rgb(60, 65, 75);
        for &t in &timestamps {
            let pos = position_at_time(t);
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
                    // Snap to nearest notch for smooth scrubbing
                    let snapped = self.graph.timeline.snap_to_notch(new_pos);
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
                let snapped = self.graph.timeline.snap_to_notch(new_pos);
                self.graph.timeline.position = snapped.max(self.graph.timeline.start_position + 0.01);
                self.graph.update_visible_nodes();
            }
        }
    }
}

impl eframe::App for DashboardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_fps();
        self.maybe_save_settings();

        // Check for point-in-time summary result from background thread
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

        // Check for session summary result from background thread
        if let Some(ref rx) = self.session_summary_receiver {
            match rx.try_recv() {
                Ok(Ok(data)) => {
                    self.session_summary_data = Some(data);
                    self.session_summary_loading = false;
                    self.session_summary_receiver = None;
                }
                Ok(Err(_)) => {
                    // Session summary failed, but don't show error (it's optional)
                    self.session_summary_loading = false;
                    self.session_summary_receiver = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // Still loading, request repaint to check again
                    ctx.request_repaint();
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.session_summary_loading = false;
                    self.session_summary_receiver = None;
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
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Force save settings on exit
        if self.settings_dirty {
            self.sync_settings_from_ui();
            self.settings.save();
        }
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
