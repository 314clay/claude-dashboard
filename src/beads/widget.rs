//! Beads panel widget with virtual scrolling.
//!
//! Implements virtual scrolling for efficient rendering of long lists.

use super::loader::{BeadEntry, BeadLoader};
use crate::theme;
use eframe::egui::{self, Color32, RichText, ScrollArea, Ui};

/// Row height for virtual scrolling calculations.
const ROW_HEIGHT: f32 = 48.0;
/// Maximum visible items before enabling virtual scrolling.
const VIRTUAL_SCROLL_THRESHOLD: usize = 50;

/// Render the beads panel with lazy loading and virtual scrolling.
pub fn render_beads_panel(
    ui: &mut Ui,
    loader: &mut BeadLoader,
    panel_visible: bool,
) {
    // Header
    ui.horizontal(|ui| {
        ui.heading("Beads");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new("B to toggle")
                    .small()
                    .color(theme::text::MUTED)
            );
        });
    });
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(8.0);

    // Lazy loading: only load data when panel is visible
    if !panel_visible {
        ui.label(
            RichText::new("Panel hidden")
                .color(theme::text::MUTED)
                .italics()
        );
        return;
    }

    // Check if we need to refresh the cache
    if loader.needs_refresh() {
        let result = loader.load();
        if result.parse_errors > 0 {
            ui.colored_label(
                theme::state::WARNING,
                format!("{} parse errors", result.parse_errors)
            );
        }
    }

    let total = loader.total_count();
    if total == 0 {
        ui.label(
            RichText::new("No beads found")
                .color(theme::text::MUTED)
                .italics()
        );
        return;
    }

    // Stats summary
    let groups = loader.grouped_by_status();
    ui.horizontal(|ui| {
        if !groups.in_progress.is_empty() {
            ui.colored_label(
                Color32::from_rgb(59, 130, 246),
                format!("{} in progress", groups.in_progress.len())
            );
        }
        if !groups.open.is_empty() {
            ui.colored_label(
                Color32::from_rgb(34, 197, 94),
                format!("{} open", groups.open.len())
            );
        }
        if !groups.blocked.is_empty() {
            ui.colored_label(
                Color32::from_rgb(239, 68, 68),
                format!("{} blocked", groups.blocked.len())
            );
        }
    });
    ui.add_space(8.0);

    // Decide whether to use virtual scrolling
    let use_virtual = total > VIRTUAL_SCROLL_THRESHOLD;

    if use_virtual {
        render_virtual_scrolling(ui, loader);
    } else {
        render_simple_list(ui, loader);
    }
}

/// Render with simple ScrollArea (for small lists).
fn render_simple_list(ui: &mut Ui, loader: &BeadLoader) {
    ScrollArea::vertical()
        .id_salt("beads_simple_scroll")
        .show(ui, |ui| {
            let groups = loader.grouped_by_status();

            // In Progress section
            if !groups.in_progress.is_empty() {
                render_section(ui, "In Progress", &groups.in_progress);
            }

            // Open section
            if !groups.open.is_empty() {
                render_section(ui, "Open", &groups.open);
            }

            // Blocked section
            if !groups.blocked.is_empty() {
                render_section(ui, "Blocked", &groups.blocked);
            }

            // Hooked section
            if !groups.hooked.is_empty() {
                render_section(ui, "Hooked", &groups.hooked);
            }

            // Closed section (collapsed by default)
            if !groups.closed.is_empty() {
                egui::CollapsingHeader::new(
                    RichText::new(format!("Closed ({})", groups.closed.len()))
                        .color(theme::text::MUTED)
                )
                .default_open(false)
                .show(ui, |ui| {
                    for entry in &groups.closed {
                        render_bead_item(ui, entry);
                    }
                });
            }
        });
}

/// Render a section header and items.
fn render_section(ui: &mut Ui, title: &str, items: &[&BeadEntry]) {
    ui.add_space(4.0);
    ui.label(RichText::new(title).strong());
    ui.add_space(4.0);

    for entry in items {
        render_bead_item(ui, entry);
    }

    ui.add_space(8.0);
}

/// Render with virtual scrolling (for large lists).
fn render_virtual_scrolling(ui: &mut Ui, loader: &BeadLoader) {
    let entries = loader.get_entries();
    let total_height = entries.len() as f32 * ROW_HEIGHT;

    ScrollArea::vertical()
        .id_salt("beads_virtual_scroll")
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            // Calculate visible range
            let first_visible = (viewport.min.y / ROW_HEIGHT).floor() as usize;
            let visible_count = ((viewport.height() / ROW_HEIGHT).ceil() as usize) + 2;
            let last_visible = (first_visible + visible_count).min(entries.len());

            // Add spacing for items before visible range
            if first_visible > 0 {
                ui.add_space(first_visible as f32 * ROW_HEIGHT);
            }

            // Render only visible items
            for entry in entries.iter().skip(first_visible).take(last_visible - first_visible) {
                render_bead_item_fixed_height(ui, entry);
            }

            // Add spacing for items after visible range
            let remaining = entries.len().saturating_sub(last_visible);
            if remaining > 0 {
                ui.add_space(remaining as f32 * ROW_HEIGHT);
            }

            // Set minimum size to enable scrolling
            ui.set_min_height(total_height);
        });
}

/// Render a single bead item.
fn render_bead_item(ui: &mut Ui, entry: &BeadEntry) {
    ui.horizontal(|ui| {
        // Status badge
        let status_color = entry.status_color();
        ui.add(
            egui::Label::new(
                RichText::new(&entry.status)
                    .small()
                    .color(Color32::WHITE)
            )
            .wrap_mode(egui::TextWrapMode::Truncate)
        );
        ui.painter().rect_filled(
            ui.min_rect(),
            4.0,
            status_color.linear_multiply(0.3),
        );

        // ID
        ui.label(
            RichText::new(&entry.id)
                .small()
                .color(theme::text::MUTED)
        );

        // Title (truncated)
        let title = if entry.title.len() > 40 {
            format!("{}...", &entry.title[..37])
        } else {
            entry.title.clone()
        };
        ui.label(RichText::new(title).small());
    });
}

/// Render a single bead item with fixed height for virtual scrolling.
fn render_bead_item_fixed_height(ui: &mut Ui, entry: &BeadEntry) {
    let (rect, _response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Sense::hover()
    );

    if ui.is_rect_visible(rect) {
        let painter = ui.painter_at(rect);
        let status_color = entry.status_color();

        // Background with subtle status color
        painter.rect_filled(
            rect.shrink(1.0),
            4.0,
            status_color.linear_multiply(0.1),
        );

        // Status badge (left side)
        let badge_rect = egui::Rect::from_min_size(
            rect.min + egui::vec2(4.0, 4.0),
            egui::vec2(60.0, 16.0),
        );
        painter.rect_filled(badge_rect, 3.0, status_color.linear_multiply(0.3));
        painter.text(
            badge_rect.center(),
            egui::Align2::CENTER_CENTER,
            &entry.status,
            egui::FontId::proportional(10.0),
            Color32::WHITE,
        );

        // ID (small, muted)
        painter.text(
            rect.min + egui::vec2(70.0, 8.0),
            egui::Align2::LEFT_TOP,
            &entry.id,
            egui::FontId::proportional(10.0),
            theme::text::MUTED,
        );

        // Title (main line)
        let title = if entry.title.len() > 50 {
            format!("{}...", &entry.title[..47])
        } else {
            entry.title.clone()
        };
        painter.text(
            rect.min + egui::vec2(4.0, 24.0),
            egui::Align2::LEFT_TOP,
            title,
            egui::FontId::proportional(12.0),
            theme::text::SECONDARY,
        );

        // Priority indicator (right side)
        if entry.priority > 0 && entry.priority <= 3 {
            let priority_text = format!("P{}", entry.priority);
            let priority_color = match entry.priority {
                1 => Color32::from_rgb(239, 68, 68),   // Red for P1
                2 => Color32::from_rgb(245, 158, 11), // Orange for P2
                _ => Color32::from_rgb(34, 197, 94),  // Green for P3+
            };
            painter.text(
                rect.right_top() + egui::vec2(-24.0, 8.0),
                egui::Align2::RIGHT_TOP,
                priority_text,
                egui::FontId::proportional(10.0),
                priority_color,
            );
        }
    }
}
