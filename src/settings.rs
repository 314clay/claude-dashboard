//! Persistent settings for the dashboard app.

use crate::graph::types::RoleFilter;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// All persistable UI settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    // Data Selection
    pub time_range_hours: f32,

    // Display
    pub node_size: f32,
    pub show_arrows: bool,
    pub timeline_enabled: bool,
    pub size_by_importance: bool,
    pub color_by_project: bool,
    pub timeline_spacing_even: bool,
    pub timeline_speed: f32,

    // Filtering
    pub importance_threshold: f32,
    pub importance_filter_enabled: bool,
    #[serde(default)]
    pub role_filter: RoleFilter,

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

    // Recency Scaling
    pub recency_min_scale: f32,
    pub recency_decay_rate: f32,
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
            size_by_importance: false,
            color_by_project: true,
            timeline_spacing_even: false,
            timeline_speed: 1.0,

            // Filtering
            importance_threshold: 0.0,
            importance_filter_enabled: false,
            role_filter: RoleFilter::default(),

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

            // Recency Scaling
            recency_min_scale: 0.01,
            recency_decay_rate: 3.0,
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
