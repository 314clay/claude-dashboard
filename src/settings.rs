//! Persistent settings for the dashboard app.

use crate::graph::types::{ColorMode, ViewMode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Per-view physics and sizing settings
/// These are the settings that change when switching between view modes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewSettings {
    // Physics
    pub repulsion: f32,
    pub attraction: f32,
    pub centering: f32,
    pub temporal_strength: f32,
    pub size_physics_weight: f32,
    pub temporal_attraction_enabled: bool,
    pub temporal_window_mins: f32,
    pub temporal_edge_opacity: f32,
    pub max_temporal_edges: usize,

    // For timeline view: vertical repulsion multiplier
    // (multiplied with repulsion for Y-axis spreading)
    pub vertical_repulsion_multiplier: f32,

    // Node sizing weights
    pub w_importance: f32,
    pub w_tokens: f32,
    pub w_time: f32,
    pub max_node_multiplier: f32,
}

impl ViewSettings {
    /// Default settings for force-directed view (current tuned values)
    pub fn force_directed_defaults() -> Self {
        Self {
            // Physics - current tuned values
            repulsion: 10000.0,
            attraction: 0.1,
            centering: 0.0001,
            temporal_strength: 0.5,
            size_physics_weight: 0.0,
            temporal_attraction_enabled: true,
            temporal_window_mins: 5.0,
            temporal_edge_opacity: 0.3,
            max_temporal_edges: 100_000,

            // Standard vertical repulsion (no multiplier)
            vertical_repulsion_multiplier: 1.0,

            // Node sizing - balanced defaults
            w_importance: 0.5,
            w_tokens: 0.3,
            w_time: 0.5,
            max_node_multiplier: 10.0,
        }
    }

    /// Default settings for timeline view
    /// - Temporal edges OFF (X axis already encodes time)
    /// - Stronger vertical repulsion (to spread nodes vertically)
    /// - Different sizing weights (time less important since X shows time)
    pub fn timeline_defaults() -> Self {
        Self {
            // Physics tuned for timeline
            repulsion: 10000.0,
            attraction: 0.05, // Weaker attraction - less clustering
            centering: 0.0001,
            temporal_strength: 0.0, // Not used
            size_physics_weight: 0.0,
            temporal_attraction_enabled: false, // OFF - X axis is time
            temporal_window_mins: 5.0,
            temporal_edge_opacity: 0.0, // Hidden
            max_temporal_edges: 0,

            // Stronger vertical repulsion to spread nodes apart
            vertical_repulsion_multiplier: 2.5,

            // Node sizing - importance and tokens matter more, time less
            // (since temporal position is already shown on X axis)
            w_importance: 0.7,
            w_tokens: 0.5,
            w_time: 0.1, // Lower - time is shown via X position
            max_node_multiplier: 8.0,
        }
    }

    /// Get defaults for a given view mode
    pub fn defaults_for(mode: ViewMode) -> Self {
        match mode {
            ViewMode::ForceDirected => Self::force_directed_defaults(),
            ViewMode::Timeline => Self::timeline_defaults(),
        }
    }
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self::force_directed_defaults()
    }
}

/// Preset configurations for node sizing formula
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SizingPreset {
    #[default]
    Balanced,
    ImportanceFocused,
    RecencyFocused,
    TokenFocused,
    Uniform,
    Custom,
}

impl SizingPreset {
    /// Get display label for the preset
    pub fn label(&self) -> &'static str {
        match self {
            SizingPreset::Balanced => "Balanced",
            SizingPreset::ImportanceFocused => "Importance-focused",
            SizingPreset::RecencyFocused => "Recency-focused",
            SizingPreset::TokenFocused => "Token-focused",
            SizingPreset::Uniform => "Uniform",
            SizingPreset::Custom => "Custom",
        }
    }

    /// Get the weight values for this preset (w_imp, w_tok, w_time)
    pub fn weights(&self) -> (f32, f32, f32) {
        match self {
            SizingPreset::Balanced => (0.5, 0.3, 0.5),
            SizingPreset::ImportanceFocused => (1.0, 0.1, 0.2),
            SizingPreset::RecencyFocused => (0.2, 0.1, 1.0),
            SizingPreset::TokenFocused => (0.1, 1.0, 0.2),
            SizingPreset::Uniform => (0.0, 0.0, 0.0),
            SizingPreset::Custom => (0.5, 0.3, 0.5), // Default values for custom
        }
    }

    /// All presets for UI iteration (excludes Custom since it's auto-selected)
    pub fn all() -> &'static [SizingPreset] {
        &[
            SizingPreset::Balanced,
            SizingPreset::ImportanceFocused,
            SizingPreset::RecencyFocused,
            SizingPreset::TokenFocused,
            SizingPreset::Uniform,
        ]
    }
}

/// A saved preset of display/physics settings (excludes data selection)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,

    // Display
    pub node_size: f32,
    pub show_arrows: bool,
    pub timeline_enabled: bool,
    #[serde(default)]
    pub color_mode: ColorMode,
    pub timeline_speed: f32,

    // Node Sizing
    pub sizing_preset: SizingPreset,
    pub w_importance: f32,
    pub w_tokens: f32,
    pub w_time: f32,
    pub max_node_multiplier: f32,

    // Filtering
    pub importance_threshold: f32,
    pub importance_filter_enabled: bool,

    // Physics
    pub physics_enabled: bool,
    pub repulsion: f32,
    pub attraction: f32,
    pub centering: f32,
    #[serde(default)]
    pub size_physics_weight: f32,
    pub temporal_strength: f32,
    pub temporal_attraction_enabled: bool,
    pub temporal_window_mins: f32,
    pub temporal_edge_opacity: f32,
    pub max_temporal_edges: usize,

    // Color Snapshot
    #[serde(default)]
    pub hue_offset: f32,
    #[serde(default)]
    pub project_colors: HashMap<String, f32>,
    #[serde(default)]
    pub session_colors: HashMap<String, f32>,
}

impl Preset {
    /// Create a preset from current settings and graph state
    pub fn from_settings(
        name: String,
        settings: &Settings,
        graph: &crate::graph::types::GraphState,
    ) -> Self {
        Self {
            name,
            node_size: settings.node_size,
            show_arrows: settings.show_arrows,
            timeline_enabled: settings.timeline_enabled,
            color_mode: settings.color_mode,
            timeline_speed: settings.timeline_speed,
            sizing_preset: settings.sizing_preset,
            w_importance: settings.w_importance,
            w_tokens: settings.w_tokens,
            w_time: settings.w_time,
            max_node_multiplier: settings.max_node_multiplier,
            importance_threshold: settings.importance_threshold,
            importance_filter_enabled: settings.importance_filter_enabled,
            physics_enabled: settings.physics_enabled,
            repulsion: settings.repulsion,
            attraction: settings.attraction,
            centering: settings.centering,
            size_physics_weight: settings.size_physics_weight,
            temporal_strength: settings.temporal_strength,
            temporal_attraction_enabled: settings.temporal_attraction_enabled,
            temporal_window_mins: settings.temporal_window_mins,
            temporal_edge_opacity: settings.temporal_edge_opacity,
            max_temporal_edges: settings.max_temporal_edges,
            // Color snapshot
            hue_offset: graph.hue_offset,
            project_colors: graph.project_colors.clone(),
            session_colors: graph.session_colors.clone(),
        }
    }

    /// Apply this preset to settings and restore colors to graph
    pub fn apply_to(&self, settings: &mut Settings, graph: &mut crate::graph::types::GraphState) {
        settings.node_size = self.node_size;
        settings.show_arrows = self.show_arrows;
        settings.timeline_enabled = self.timeline_enabled;
        settings.color_mode = self.color_mode;
        settings.timeline_speed = self.timeline_speed;
        settings.sizing_preset = self.sizing_preset;
        settings.w_importance = self.w_importance;
        settings.w_tokens = self.w_tokens;
        settings.w_time = self.w_time;
        settings.max_node_multiplier = self.max_node_multiplier;
        settings.importance_threshold = self.importance_threshold;
        settings.importance_filter_enabled = self.importance_filter_enabled;
        settings.physics_enabled = self.physics_enabled;
        settings.repulsion = self.repulsion;
        settings.attraction = self.attraction;
        settings.centering = self.centering;
        settings.size_physics_weight = self.size_physics_weight;
        settings.temporal_strength = self.temporal_strength;
        settings.temporal_attraction_enabled = self.temporal_attraction_enabled;
        settings.temporal_window_mins = self.temporal_window_mins;
        settings.temporal_edge_opacity = self.temporal_edge_opacity;
        settings.max_temporal_edges = self.max_temporal_edges;

        // Restore colors (merge: saved colors take precedence over current)
        graph.hue_offset = self.hue_offset;
        for (k, v) in &self.project_colors {
            graph.project_colors.insert(k.clone(), *v);
        }
        for (k, v) in &self.session_colors {
            graph.session_colors.insert(k.clone(), *v);
        }
    }
}

/// All persistable UI settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    // Data Selection
    pub time_range_hours: f32,

    // Display
    pub node_size: f32,
    pub show_arrows: bool,
    pub timeline_enabled: bool,
    #[serde(default)]
    pub color_mode: ColorMode,

    // Current view mode
    #[serde(default)]
    pub view_mode: ViewMode,

    // Per-view settings (stored separately so they persist across view switches)
    #[serde(default = "ViewSettings::force_directed_defaults")]
    pub force_directed_settings: ViewSettings,
    #[serde(default = "ViewSettings::timeline_defaults")]
    pub timeline_view_settings: ViewSettings,

    // Node Sizing (unified formula) - these are the ACTIVE values, updated from view settings
    #[serde(default)]
    pub sizing_preset: SizingPreset,
    #[serde(default = "default_w_importance")]
    pub w_importance: f32,
    #[serde(default = "default_w_tokens")]
    pub w_tokens: f32,
    #[serde(default = "default_w_time")]
    pub w_time: f32,
    #[serde(default = "default_max_node_multiplier")]
    pub max_node_multiplier: f32,
    #[serde(default)]
    pub timeline_spacing_even: bool,
    #[serde(default = "default_timeline_speed")]
    pub timeline_speed: f32,
    #[serde(default = "default_hover_scrubs_timeline")]
    pub hover_scrubs_timeline: bool,

    // Filtering
    pub importance_threshold: f32,
    pub importance_filter_enabled: bool,

    // Physics - these are the ACTIVE values, updated from view settings
    pub physics_enabled: bool,
    pub repulsion: f32,
    pub attraction: f32,
    pub centering: f32,
    /// How much visual size affects physics (0 = uniform, higher = more differentiation)
    pub size_physics_weight: f32,
    pub temporal_strength: f32,
    pub temporal_attraction_enabled: bool,
    pub temporal_window_mins: f32,
    pub temporal_edge_opacity: f32,
    #[serde(default = "default_max_temporal_edges")]
    pub max_temporal_edges: usize,

    // Saved presets
    #[serde(default)]
    pub presets: Vec<Preset>,

    // Refresh & sync
    #[serde(default = "default_auto_refresh_enabled")]
    pub auto_refresh_enabled: bool,
    #[serde(default = "default_auto_refresh_interval_secs")]
    pub auto_refresh_interval_secs: f32,

    // Panel visibility (collapsible side panels)
    #[serde(default = "default_beads_panel_open")]
    pub beads_panel_open: bool,
    #[serde(default = "default_mail_panel_open")]
    pub mail_panel_open: bool,
}

impl Settings {
    /// Get the view settings for the current view mode
    pub fn active_view_settings(&self) -> &ViewSettings {
        match self.view_mode {
            ViewMode::ForceDirected => &self.force_directed_settings,
            ViewMode::Timeline => &self.timeline_view_settings,
        }
    }

    /// Get mutable view settings for the current view mode
    pub fn active_view_settings_mut(&mut self) -> &mut ViewSettings {
        match self.view_mode {
            ViewMode::ForceDirected => &mut self.force_directed_settings,
            ViewMode::Timeline => &mut self.timeline_view_settings,
        }
    }

    /// Apply the active view's settings to the main settings fields
    /// Call this after switching view modes
    pub fn apply_active_view_settings(&mut self) {
        let view = self.active_view_settings().clone();
        self.repulsion = view.repulsion;
        self.attraction = view.attraction;
        self.centering = view.centering;
        self.temporal_strength = view.temporal_strength;
        self.size_physics_weight = view.size_physics_weight;
        self.temporal_attraction_enabled = view.temporal_attraction_enabled;
        self.temporal_window_mins = view.temporal_window_mins;
        self.temporal_edge_opacity = view.temporal_edge_opacity;
        self.max_temporal_edges = view.max_temporal_edges;
        self.w_importance = view.w_importance;
        self.w_tokens = view.w_tokens;
        self.w_time = view.w_time;
        self.max_node_multiplier = view.max_node_multiplier;
    }

    /// Save current main settings back to the active view's settings
    /// Call this before switching view modes to preserve changes
    pub fn save_to_active_view_settings(&mut self) {
        // Read existing vertical_repulsion_multiplier before building new settings
        let vertical_mult = match self.view_mode {
            ViewMode::ForceDirected => self.force_directed_settings.vertical_repulsion_multiplier,
            ViewMode::Timeline => self.timeline_view_settings.vertical_repulsion_multiplier,
        };
        let view_settings = ViewSettings {
            repulsion: self.repulsion,
            attraction: self.attraction,
            centering: self.centering,
            temporal_strength: self.temporal_strength,
            size_physics_weight: self.size_physics_weight,
            temporal_attraction_enabled: self.temporal_attraction_enabled,
            temporal_window_mins: self.temporal_window_mins,
            temporal_edge_opacity: self.temporal_edge_opacity,
            max_temporal_edges: self.max_temporal_edges,
            vertical_repulsion_multiplier: vertical_mult,
            w_importance: self.w_importance,
            w_tokens: self.w_tokens,
            w_time: self.w_time,
            max_node_multiplier: self.max_node_multiplier,
        };
        match self.view_mode {
            ViewMode::ForceDirected => self.force_directed_settings = view_settings,
            ViewMode::Timeline => self.timeline_view_settings = view_settings,
        }
    }
}

fn default_timeline_speed() -> f32 {
    1.0
}

fn default_hover_scrubs_timeline() -> bool {
    true
}

fn default_max_temporal_edges() -> usize {
    100_000
}

fn default_w_importance() -> f32 {
    0.5
}

fn default_w_tokens() -> f32 {
    0.3
}

fn default_w_time() -> f32 {
    0.5
}

fn default_max_node_multiplier() -> f32 {
    10.0
}

fn default_auto_refresh_enabled() -> bool {
    false
}

fn default_auto_refresh_interval_secs() -> f32 {
    5.0
}

fn default_beads_panel_open() -> bool {
    false
}

fn default_mail_panel_open() -> bool {
    false
}

impl Default for Settings {
    fn default() -> Self {
        let force_directed = ViewSettings::force_directed_defaults();
        Self {
            // Data Selection
            time_range_hours: 24.0,

            // Display
            node_size: 15.0,
            show_arrows: true,
            timeline_enabled: true,
            color_mode: ColorMode::Project,
            timeline_spacing_even: false,
            timeline_speed: 1.0,
            hover_scrubs_timeline: true,

            // View mode
            view_mode: ViewMode::ForceDirected,
            force_directed_settings: ViewSettings::force_directed_defaults(),
            timeline_view_settings: ViewSettings::timeline_defaults(),

            // Node Sizing (initialized from force-directed defaults)
            sizing_preset: SizingPreset::Balanced,
            w_importance: force_directed.w_importance,
            w_tokens: force_directed.w_tokens,
            w_time: force_directed.w_time,
            max_node_multiplier: force_directed.max_node_multiplier,

            // Filtering
            importance_threshold: 0.0,
            importance_filter_enabled: false,

            // Physics (initialized from force-directed defaults)
            physics_enabled: true,
            repulsion: force_directed.repulsion,
            attraction: force_directed.attraction,
            centering: force_directed.centering,
            size_physics_weight: force_directed.size_physics_weight,
            temporal_strength: force_directed.temporal_strength,
            temporal_attraction_enabled: force_directed.temporal_attraction_enabled,
            temporal_window_mins: force_directed.temporal_window_mins,
            temporal_edge_opacity: force_directed.temporal_edge_opacity,
            max_temporal_edges: force_directed.max_temporal_edges,

            // Presets
            presets: Vec::new(),

            // Refresh & sync
            auto_refresh_enabled: false,
            auto_refresh_interval_secs: 5.0,

            // Panel visibility
            beads_panel_open: false,
            mail_panel_open: false,
        }
    }
}

impl Settings {
    /// Get the path to the settings file
    fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|mut p| {
            p.push("dashboard-native");
            p.push("settings.json");
            p
        })
    }

    /// Load settings from disk, returning defaults if file doesn't exist or is invalid
    pub fn load() -> Self {
        let Some(path) = Self::config_path() else {
            eprintln!("Could not determine config directory, using defaults");
            return Self::default();
        };

        match std::fs::read_to_string(&path) {
            Ok(contents) => {
                match serde_json::from_str(&contents) {
                    Ok(settings) => {
                        eprintln!("Loaded settings from {:?}", path);
                        settings
                    }
                    Err(e) => {
                        eprintln!("Failed to parse settings file: {}, using defaults", e);
                        Self::default()
                    }
                }
            }
            Err(_) => {
                // File doesn't exist yet, that's fine
                Self::default()
            }
        }
    }

    /// Save settings to disk
    pub fn save(&self) {
        let Some(path) = Self::config_path() else {
            eprintln!("Could not determine config directory, settings not saved");
            return;
        };

        // Ensure config directory exists
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("Failed to create config directory: {}", e);
                return;
            }
        }

        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    eprintln!("Failed to write settings file: {}", e);
                } else {
                    eprintln!("Saved settings to {:?}", path);
                }
            }
            Err(e) => {
                eprintln!("Failed to serialize settings: {}", e);
            }
        }
    }
}
