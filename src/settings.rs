//! Persistent settings for the dashboard app.

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

/// All persistable UI settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    // Data Selection
    pub time_range_hours: f32,

    // Display
    pub node_size: f32,
    pub show_arrows: bool,
    pub timeline_enabled: bool,
    pub color_by_project: bool,

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
    pub size_repulsion_weight: f32,
    pub temporal_strength: f32,
    pub temporal_attraction_enabled: bool,
    pub temporal_window_mins: f32,
    pub temporal_edge_opacity: f32,
    #[serde(default = "default_max_temporal_edges")]
    pub max_temporal_edges: usize,
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
            color_by_project: true,
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
            size_repulsion_weight: 0.0,
            temporal_strength: 0.5,
            temporal_attraction_enabled: true,
            temporal_window_mins: 5.0,
            temporal_edge_opacity: 0.3,
            max_temporal_edges: 100_000,
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
