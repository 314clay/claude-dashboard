//! Main application state and UI.

use crate::api::ApiClient;
use crate::graph::{ForceLayout, GraphState};
use eframe::egui::{self, Color32, Pos2, Stroke, Vec2};
use std::time::Instant;

/// Time range options for filtering
#[derive(Debug, Clone, Copy, PartialEq)]
enum TimeRange {
    Hour1,
    Hour6,
    Hour24,
    Day3,
    Week1,
}

impl TimeRange {
    fn hours(&self) -> f32 {
        match self {
            TimeRange::Hour1 => 1.0,
            TimeRange::Hour6 => 6.0,
            TimeRange::Hour24 => 24.0,
            TimeRange::Day3 => 72.0,
            TimeRange::Week1 => 168.0,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            TimeRange::Hour1 => "Past hour",
            TimeRange::Hour6 => "Past 6 hours",
            TimeRange::Hour24 => "Past 24 hours",
            TimeRange::Day3 => "Past 3 days",
            TimeRange::Week1 => "Past week",
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
}

impl DashboardApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self {
            api: ApiClient::new(),
            api_connected: false,
            api_error: None,
            graph: GraphState::new(),
            layout: ForceLayout::default(),
            time_range: TimeRange::Hour24,
            node_size: 15.0,
            show_arrows: true,
            loading: false,
            timeline_enabled: true,
            recency_min_scale: 0.01,
            recency_decay_rate: 3.0,
            pan_offset: Vec2::ZERO,
            zoom: 1.0,
            dragging: false,
            drag_start: None,
            timeline_dragging: false,
            last_playback_time: Instant::now(),
            last_frame: Instant::now(),
            frame_times: Vec::with_capacity(60),
            fps: 0.0,
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
        ui.separator();

        // Time range selector
        ui.label("Time Range");
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
                ] {
                    ui.selectable_value(&mut self.time_range, range, range.label());
                }
            });

        if self.time_range != prev_range {
            self.load_graph();
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
                self.recency_min_scale = 0.3;
                self.recency_decay_rate = 3.0;
                self.layout.repulsion = 5000.0;
                self.layout.attraction = 0.01;
                self.layout.centering = 0.001;
                self.pan_offset = Vec2::ZERO;
                self.zoom = 1.0;
                self.load_graph();
            }
        });

        ui.add_space(10.0);
        ui.separator();

        // Display options
        ui.label("Display Options");
        ui.add(egui::Slider::new(&mut self.node_size, 5.0..=50.0).text("Node size"));
        ui.checkbox(&mut self.show_arrows, "Show arrows");
        ui.checkbox(&mut self.graph.physics_enabled, "Physics enabled");
        ui.checkbox(&mut self.timeline_enabled, "Timeline scrubber");

        ui.add_space(10.0);
        ui.separator();

        // Recency scaling
        ui.label("Recency Scaling");
        ui.add(egui::Slider::new(&mut self.recency_min_scale, 0.01..=1.0).text("Min scale"));
        ui.add(egui::Slider::new(&mut self.recency_decay_rate, 0.5..=10.0).text("Decay rate"));

        ui.add_space(10.0);
        ui.separator();

        // Physics controls
        ui.label("Physics");
        ui.add(egui::Slider::new(&mut self.layout.repulsion, 1000.0..=20000.0).text("Repulsion"));
        ui.add(egui::Slider::new(&mut self.layout.attraction, 0.001..=0.1).text("Attraction"));
        ui.add(egui::Slider::new(&mut self.layout.centering, 0.0001..=0.01).text("Centering"));

        ui.add_space(10.0);
        ui.separator();

        // Stats
        ui.label("Statistics");
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

        ui.add_space(10.0);
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
                let role_color = closest_node.role.color();
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

            // Content preview with word wrap
            ui.add_space(5.0);
            let preview = if closest_node.content_preview.len() > 100 {
                format!("{}...", &closest_node.content_preview[..100])
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

        // Legend
        ui.separator();
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

    fn render_graph(&mut self, ui: &mut egui::Ui) {
        let (response, painter) = ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
        let rect = response.rect;
        let center = rect.center();

        // Handle pan
        if response.dragged_by(egui::PointerButton::Primary) {
            self.pan_offset += response.drag_delta();
        }

        // Handle zoom with scroll
        if let Some(hover_pos) = response.hover_pos() {
            let scroll = ui.input(|i| i.raw_scroll_delta.y);
            if scroll != 0.0 {
                let zoom_factor = 1.0 + scroll * 0.001;
                let new_zoom = (self.zoom * zoom_factor).clamp(0.1, 5.0);

                // Zoom toward mouse position
                let mouse_offset = hover_pos - center;
                self.pan_offset = self.pan_offset * (new_zoom / self.zoom)
                    + mouse_offset * (1.0 - new_zoom / self.zoom);

                self.zoom = new_zoom;
            }
        }

        // Run physics simulation
        self.layout.step(&mut self.graph, center + self.pan_offset);

        // Transform helper
        let transform = |pos: Pos2| -> Pos2 {
            let centered = pos.to_vec2() - center.to_vec2();
            center + (centered + self.pan_offset) * self.zoom
        };

        // Draw edges first (behind nodes)
        for edge in &self.graph.data.edges {
            // Skip edges for hidden nodes when timeline is enabled
            if self.timeline_enabled && !self.graph.is_edge_visible(edge) {
                continue;
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

        // Detect hover (only for visible nodes)
        let mut new_hovered = None;
        if let Some(hover_pos) = response.hover_pos() {
            for node in &self.graph.data.nodes {
                // Skip hidden nodes when timeline is enabled
                if self.timeline_enabled && !self.graph.is_node_visible(&node.id) {
                    continue;
                }
                if let Some(pos) = self.graph.get_pos(&node.id) {
                    let screen_pos = transform(pos);
                    // Use same recency scaling for hit detection (based on distance from scrubber)
                    let min_s = self.recency_min_scale;
                    let decay = self.recency_decay_rate;
                    let recency_scale = if self.graph.timeline.max_time > self.graph.timeline.min_time {
                        if let Some(node_time) = node.timestamp_secs() {
                            let time_range = self.graph.timeline.max_time - self.graph.timeline.min_time;
                            let scrubber_time = self.graph.timeline.time_at_position(self.graph.timeline.position);
                            let distance = (scrubber_time - node_time).abs();
                            let normalized_distance = (distance / time_range).clamp(0.0, 1.0);
                            min_s + (1.0 - min_s) * (-decay * normalized_distance as f32).exp()
                        } else { 0.5 }
                    } else { 1.0 };
                    let node_radius = self.node_size * self.zoom * recency_scale;
                    if screen_pos.distance(hover_pos) < node_radius {
                        new_hovered = Some(node.id.clone());
                        break;
                    }
                }
            }
        }
        self.graph.hovered_node = new_hovered;

        // Draw nodes (only visible ones when timeline is enabled)
        for node in &self.graph.data.nodes {
            // Skip hidden nodes when timeline is enabled
            if self.timeline_enabled && !self.graph.is_node_visible(&node.id) {
                continue;
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

                let base_size = self.node_size * self.zoom * recency_scale;
                let size = if is_hovered || is_selected {
                    base_size * 1.3
                } else {
                    base_size
                };

                let color = node.role.color();

                // Draw node
                painter.circle_filled(screen_pos, size, color);

                // Draw border
                let border_color = if is_selected {
                    Color32::YELLOW
                } else if is_hovered {
                    Color32::WHITE
                } else {
                    color.gamma_multiply(0.7)
                };
                painter.circle_stroke(screen_pos, size, Stroke::new(2.0, border_color));
            }
        }

        // Handle click selection (only for visible nodes)
        if response.clicked() {
            if let Some(hover_pos) = response.interact_pointer_pos() {
                let mut clicked_node = None;
                for node in &self.graph.data.nodes {
                    // Skip hidden nodes when timeline is enabled
                    if self.timeline_enabled && !self.graph.is_node_visible(&node.id) {
                        continue;
                    }
                    if let Some(pos) = self.graph.get_pos(&node.id) {
                        let screen_pos = transform(pos);
                        // Use same recency scaling for hit detection (based on distance from scrubber)
                        let min_s = self.recency_min_scale;
                        let decay = self.recency_decay_rate;
                        let recency_scale = if self.graph.timeline.max_time > self.graph.timeline.min_time {
                            if let Some(node_time) = node.timestamp_secs() {
                                let time_range = self.graph.timeline.max_time - self.graph.timeline.min_time;
                                let scrubber_time = self.graph.timeline.time_at_position(self.graph.timeline.position);
                                let distance = (scrubber_time - node_time).abs();
                                let normalized_distance = (distance / time_range).clamp(0.0, 1.0);
                                min_s + (1.0 - min_s) * (-decay * normalized_distance as f32).exp()
                            } else { 0.5 }
                        } else { 1.0 };
                        let node_radius = self.node_size * self.zoom * recency_scale;
                        if screen_pos.distance(hover_pos) < node_radius {
                            clicked_node = Some(node.id.clone());
                            break;
                        }
                    }
                }
                self.graph.selected_node = clicked_node;
            }
        }

        // Draw tooltip for hovered node
        if let Some(ref hovered_id) = self.graph.hovered_node {
            if let Some(node) = self.graph.get_node(hovered_id) {
                if let Some(pos) = self.graph.get_pos(hovered_id) {
                    let screen_pos = transform(pos);
                    let tooltip_pos = screen_pos + Vec2::new(self.node_size * self.zoom + 10.0, 0.0);

                    // Draw tooltip background
                    let tooltip_text = format!(
                        "{}\n{}\n\nSession: {}\nProject: {}",
                        node.role.label(),
                        truncate(&node.content_preview, 80),
                        node.session_short,
                        node.project
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
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        format!("{}...", &s[..max_len])
    } else {
        s.to_string()
    }
}
