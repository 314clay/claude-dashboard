//! Unified theme and color constants for the dashboard.
//!
//! This module provides a consistent color palette that bridges
//! the graph visualization and UI panels. All colors should be
//! sourced from here to maintain visual consistency.

use egui::Color32;

/// Background colors for different layers
pub mod bg {
    use super::*;

    /// Main graph area background - darkest layer
    pub const GRAPH: Color32 = Color32::from_rgb(14, 17, 23);

    /// Panel backgrounds - slightly lighter than graph
    pub const PANEL: Color32 = Color32::from_rgb(20, 22, 28);

    /// Card/elevated surface backgrounds
    pub const SURFACE: Color32 = Color32::from_rgb(28, 30, 38);

    /// Interactive element backgrounds (buttons, inputs)
    pub const INTERACTIVE: Color32 = Color32::from_rgb(35, 38, 48);

    /// Hover state for interactive elements
    pub const INTERACTIVE_HOVER: Color32 = Color32::from_rgb(45, 48, 58);

    /// Active/pressed state for interactive elements
    pub const INTERACTIVE_ACTIVE: Color32 = Color32::from_rgb(55, 58, 68);

    /// Timeline track background
    pub const TIMELINE_TRACK: Color32 = Color32::from_rgb(30, 33, 40);
}

/// Accent colors that match the graph node roles
pub mod accent {
    use super::*;

    /// Claude/Assistant orange - primary accent
    pub const ORANGE: Color32 = Color32::from_rgb(255, 149, 0);

    /// Orange with reduced opacity for highlights/selections
    pub fn orange_subtle() -> Color32 {
        Color32::from_rgba_unmultiplied(255, 149, 0, 80)
    }

    /// Cyan for connections/links/references
    pub const CYAN: Color32 = Color32::from_rgb(6, 182, 212);

    /// Green for topics/success states
    pub const GREEN: Color32 = Color32::from_rgb(34, 197, 94);

    /// Purple for obsidian/notes
    pub const PURPLE: Color32 = Color32::from_rgb(155, 89, 182);

    /// Blue for info/secondary actions
    pub const BLUE: Color32 = Color32::from_rgb(59, 130, 246);

    /// Red for errors/exclusions
    pub const RED: Color32 = Color32::from_rgb(239, 68, 68);

    /// Yellow for selection highlighting
    pub const YELLOW: Color32 = Color32::from_rgb(255, 220, 80);
}

/// Text colors at different emphasis levels
pub mod text {
    use super::*;

    /// Primary text - high contrast
    pub const PRIMARY: Color32 = Color32::from_rgb(240, 240, 245);

    /// Secondary text - medium contrast
    pub const SECONDARY: Color32 = Color32::from_rgb(180, 180, 190);

    /// Muted text - low contrast for less important info
    pub const MUTED: Color32 = Color32::from_rgb(120, 125, 135);

    /// Disabled text
    pub const DISABLED: Color32 = Color32::from_rgb(80, 85, 95);
}

/// Border colors
pub mod border {
    use super::*;

    /// Subtle border for separators
    pub const SUBTLE: Color32 = Color32::from_rgb(45, 48, 55);

    /// Default border for cards/panels
    pub const DEFAULT: Color32 = Color32::from_rgb(55, 58, 65);

    /// Emphasized border for focused elements
    pub const FOCUS: Color32 = Color32::from_rgb(80, 85, 95);
}

/// State colors for interactive elements
pub mod state {
    use super::*;

    /// Hover state border/outline
    pub const HOVER: Color32 = Color32::WHITE;

    /// Selected state border/outline
    pub const SELECTED: Color32 = super::accent::YELLOW;

    /// Active/focused state (e.g., summary node)
    pub const ACTIVE: Color32 = super::accent::CYAN;

    /// Success indicator
    pub const SUCCESS: Color32 = super::accent::GREEN;

    /// Error indicator
    pub const ERROR: Color32 = super::accent::RED;

    /// Warning indicator
    pub const WARNING: Color32 = Color32::from_rgb(245, 158, 11);
}

/// Timeline-specific colors
pub mod timeline {
    use super::*;

    /// Inactive histogram bar
    pub const BAR_INACTIVE: Color32 = Color32::from_rgb(80, 90, 110);

    /// Highlighted/hovered histogram bar
    pub const BAR_HIGHLIGHT: Color32 = Color32::from_rgb(100, 120, 150);

    /// Bar within selected range
    pub const BAR_SELECTED: Color32 = super::accent::ORANGE;

    /// Tick marks/notches
    pub const NOTCH: Color32 = Color32::from_rgb(60, 65, 75);

    /// Start handle
    pub const HANDLE_START: Color32 = Color32::from_rgb(100, 100, 120);

    /// End handle
    pub const HANDLE_END: Color32 = super::accent::ORANGE;

    /// Playhead/current position
    pub const PLAYHEAD: Color32 = super::accent::CYAN;
}

/// Skeleton loading placeholder colors
pub mod skeleton {
    use super::*;

    /// Base skeleton background
    pub const BASE: Color32 = Color32::from_rgb(35, 38, 48);

    /// Animated shimmer highlight
    pub const SHIMMER: Color32 = Color32::from_rgb(50, 53, 63);
}

/// Semantic filter button colors
pub mod filter {
    use super::*;

    /// Inactive filter button
    pub const INACTIVE: Color32 = Color32::from_rgb(50, 50, 60);

    /// Active but neutral state
    pub const ACTIVE: Color32 = Color32::from_rgb(100, 100, 120);

    /// Include mode (green)
    pub const INCLUDE: Color32 = super::accent::GREEN;

    /// Include+1 mode (blue)
    pub const INCLUDE_PLUS1: Color32 = super::accent::BLUE;

    /// Include+2 mode (purple)
    pub const INCLUDE_PLUS2: Color32 = Color32::from_rgb(139, 92, 246);

    /// Exclude mode (red)
    pub const EXCLUDE: Color32 = super::accent::RED;
}

/// Helper to create a stroke with consistent styling
pub fn stroke(color: Color32, width: f32) -> egui::Stroke {
    egui::Stroke::new(width, color)
}

/// Node rendering stroke widths
pub mod stroke_width {
    /// Normal node border
    pub const NORMAL: f32 = 1.0;

    /// Hovered node border
    pub const HOVER: f32 = 2.0;

    /// Selected node border
    pub const SELECTED: f32 = 2.0;

    /// Active/summary node border
    pub const ACTIVE: f32 = 4.0;
}

/// Create a skeleton rectangle for loading placeholders
pub fn skeleton_rect(ui: &mut egui::Ui, width: f32, height: f32) {
    let (rect, _) = ui.allocate_exact_size(
        egui::Vec2::new(width, height),
        egui::Sense::hover(),
    );

    // Animate the shimmer effect
    let time = ui.ctx().input(|i| i.time);
    let phase = (time * 2.0).sin() * 0.5 + 0.5; // 0 to 1 oscillation

    // Interpolate between base and shimmer colors
    let color = Color32::from_rgb(
        lerp_u8(skeleton::BASE.r(), skeleton::SHIMMER.r(), phase as f32),
        lerp_u8(skeleton::BASE.g(), skeleton::SHIMMER.g(), phase as f32),
        lerp_u8(skeleton::BASE.b(), skeleton::SHIMMER.b(), phase as f32),
    );

    ui.painter().rect_filled(rect, 4.0, color);
    ui.ctx().request_repaint(); // Keep animating
}

/// Create a skeleton text line
pub fn skeleton_text(ui: &mut egui::Ui, width: f32) {
    skeleton_rect(ui, width, 14.0);
}

/// Create multiple skeleton lines (for paragraph placeholders)
pub fn skeleton_lines(ui: &mut egui::Ui, count: usize, base_width: f32) {
    for i in 0..count {
        // Vary widths for visual interest
        let width_factor = match i % 3 {
            0 => 1.0,
            1 => 0.85,
            _ => 0.7,
        };
        skeleton_text(ui, base_width * width_factor);
        if i < count - 1 {
            ui.add_space(4.0);
        }
    }
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    let result = a as f32 + (b as f32 - a as f32) * t;
    result.clamp(0.0, 255.0) as u8
}

/// Token histogram colors for stacked session bars
pub mod histogram {
    use super::*;
    pub const INPUT: Color32 = Color32::from_rgb(59, 130, 246);
    pub const OUTPUT: Color32 = Color32::from_rgb(255, 149, 0);
    pub const CACHE_READ: Color32 = Color32::from_rgb(34, 197, 94);
    pub const CACHE_CREATE: Color32 = Color32::from_rgb(155, 89, 182);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_accent_colors_match_role_colors() {
        // Verify accent colors match the Role::color() values in types.rs
        assert_eq!(accent::ORANGE, Color32::from_rgb(255, 149, 0)); // Assistant
        assert_eq!(accent::PURPLE, Color32::from_rgb(155, 89, 182)); // Obsidian
        assert_eq!(accent::GREEN, Color32::from_rgb(34, 197, 94)); // Topic
    }
}
