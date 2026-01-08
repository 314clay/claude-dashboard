//! Persistent settings for the dashboard app.

use crate::graph::types::ColorMode;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
}

impl Preset {
    /// Create a preset from current settings
    pub fn from_settings(name: String, settings: &Settings) -> Self {
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
        }
    }

    /// Apply this preset to settings
    pub fn apply_to(&self, settings: &mut Settings) {
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

    // Node Sizing (unified formula)
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

    // Filtering
    pub importance_threshold: f32,
    pub importance_filter_enabled: bool,

    // Physics
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
}

fn default_timeline_speed() -> f32 {
    1.0
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

impl Default for Settings {
    fn default() -> Self {
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

            // Node Sizing
            sizing_preset: SizingPreset::Balanced,
            w_importance: 0.5,
            w_tokens: 0.3,
            w_time: 0.5,
            max_node_multiplier: 10.0,

            // Filtering
            importance_threshold: 0.0,
            importance_filter_enabled: false,

            // Physics
            physics_enabled: true,
            repulsion: 10000.0,
            attraction: 0.1,
            centering: 0.0001,
            size_physics_weight: 0.0,
            temporal_strength: 0.5,
            temporal_attraction_enabled: true,
            temporal_window_mins: 5.0,
            temporal_edge_opacity: 0.3,
            max_temporal_edges: 100_000,

            // Presets
            presets: Vec::new(),
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
