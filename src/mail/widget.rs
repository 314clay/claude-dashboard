//! Mail network graph widget for sidebar display.

use egui::{Color32, Painter, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};

use super::types::MailNetworkState;

/// Color palette for agent types
fn agent_color(agent_id: &str) -> Color32 {
    // Color by rig/type
    if agent_id == "mayor" {
        Color32::from_rgb(255, 215, 0) // Gold for mayor
    } else if agent_id.contains("/witness") {
        Color32::from_rgb(147, 112, 219) // Purple for witnesses
    } else if agent_id.contains("/refinery") {
        Color32::from_rgb(255, 140, 0) // Orange for refineries
    } else if agent_id.starts_with("overseer") || agent_id.starts_with("gt-") {
        Color32::from_rgb(100, 149, 237) // Cornflower blue for system
    } else {
        // Hash the rig name for consistent colors
        let rig = agent_id.split('/').next().unwrap_or(agent_id);
        let hash = rig.bytes().fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
        let hue = (hash % 360) as f32;
        hsl_to_rgb(hue, 0.6, 0.5)
    }
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> Color32 {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;

    let (r, g, b) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    Color32::from_rgb(
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

/// Render the mail network graph widget.
pub fn render_mail_network(
    ui: &mut Ui,
    state: &mut MailNetworkState,
    size: Vec2,
) -> Response {
    let (response, painter) = ui.allocate_painter(size, Sense::click_and_drag());
    let rect = response.rect;
    let center = rect.center();

    // Background
    painter.rect_filled(rect, 4.0, Color32::from_rgb(25, 28, 35));
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0, Color32::from_rgb(60, 65, 75)));

    // Handle empty state
    if state.data.nodes.is_empty() {
        painter.text(
            center,
            egui::Align2::CENTER_CENTER,
            "No mail data",
            egui::FontId::proportional(12.0),
            Color32::GRAY,
        );
        return response;
    }

    // Run physics simulation
    state.step(center, rect, 0.016);

    // Handle dragging
    if let Some(pointer_pos) = response.interact_pointer_pos() {
        if response.drag_started() {
            // Find node under pointer
            for node in &state.data.nodes {
                if let Some(pos) = state.positions.get(&node.id) {
                    let node_radius = node_radius(node.message_count, &state.data);
                    if pos.distance(pointer_pos) <= node_radius + 5.0 {
                        state.dragged_node = Some(node.id.clone());
                        state.drag_offset = *pos - pointer_pos;
                        break;
                    }
                }
            }
        }

        if response.dragged() {
            if let Some(ref dragged_id) = state.dragged_node {
                if let Some(pos) = state.positions.get_mut(dragged_id) {
                    *pos = pointer_pos + state.drag_offset;
                    pos.x = pos.x.clamp(rect.left() + 10.0, rect.right() - 10.0);
                    pos.y = pos.y.clamp(rect.top() + 10.0, rect.bottom() - 10.0);
                }
                // Zero velocity when dragging
                if let Some(vel) = state.velocities.get_mut(dragged_id) {
                    *vel = Vec2::ZERO;
                }
            }
        }
    }

    if response.drag_stopped() {
        state.dragged_node = None;
    }

    // Handle hover
    state.hovered_node = None;
    if let Some(pointer_pos) = ui.input(|i| i.pointer.hover_pos()) {
        if rect.contains(pointer_pos) {
            for node in &state.data.nodes {
                if let Some(pos) = state.positions.get(&node.id) {
                    let node_radius = node_radius(node.message_count, &state.data);
                    if pos.distance(pointer_pos) <= node_radius + 3.0 {
                        state.hovered_node = Some(node.id.clone());
                        break;
                    }
                }
            }
        }
    }

    // Draw edges first (behind nodes)
    for edge in &state.data.edges {
        let src_pos = state.positions.get(&edge.source);
        let tgt_pos = state.positions.get(&edge.target);

        if let (Some(src), Some(tgt)) = (src_pos, tgt_pos) {
            // Edge thickness based on message count
            let thickness = 0.5 + edge.weight * 2.0;

            // Dim edges not connected to hovered node
            let alpha = if let Some(ref hovered) = state.hovered_node {
                if &edge.source == hovered || &edge.target == hovered {
                    200
                } else {
                    40
                }
            } else {
                100
            };

            let color = Color32::from_rgba_unmultiplied(150, 150, 150, alpha);
            painter.line_segment([*src, *tgt], Stroke::new(thickness, color));

            // Draw arrow head
            if edge.message_count > 0 {
                draw_arrow_head(&painter, *src, *tgt, color, thickness);
            }
        }
    }

    // Draw nodes
    for node in &state.data.nodes {
        if let Some(pos) = state.positions.get(&node.id) {
            let radius = node_radius(node.message_count, &state.data);
            let color = agent_color(&node.id);

            // Highlight hovered node
            let is_hovered = state.hovered_node.as_ref() == Some(&node.id);
            let is_connected = if let Some(ref hovered) = state.hovered_node {
                state.data.edges.iter().any(|e| {
                    (&e.source == hovered && &e.target == &node.id)
                        || (&e.target == hovered && &e.source == &node.id)
                })
            } else {
                false
            };

            let alpha = if is_hovered {
                255
            } else if state.hovered_node.is_some() && !is_connected {
                80
            } else {
                220
            };

            let node_color = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha);

            // Draw node circle
            painter.circle_filled(*pos, radius, node_color);

            // Draw border
            let border_color = if is_hovered {
                Color32::WHITE
            } else {
                Color32::from_rgba_unmultiplied(255, 255, 255, 60)
            };
            painter.circle_stroke(*pos, radius, Stroke::new(1.0, border_color));

            // Draw label for hovered or large nodes
            if is_hovered || node.message_count > 5 {
                let label_pos = Pos2::new(pos.x, pos.y - radius - 8.0);
                let font = egui::FontId::proportional(if is_hovered { 11.0 } else { 9.0 });
                let text_color = if is_hovered {
                    Color32::WHITE
                } else {
                    Color32::from_rgb(180, 180, 180)
                };

                painter.text(
                    label_pos,
                    egui::Align2::CENTER_BOTTOM,
                    &node.label,
                    font,
                    text_color,
                );
            }
        }
    }

    // Show tooltip for hovered node
    if let Some(ref hovered_id) = state.hovered_node {
        if let Some(node) = state.data.nodes.iter().find(|n| &n.id == hovered_id) {
            if let Some(pos) = state.positions.get(hovered_id) {
                let tooltip_pos = Pos2::new(pos.x + 15.0, pos.y - 10.0);
                let tooltip_rect = Rect::from_min_size(tooltip_pos, Vec2::new(150.0, 50.0));

                // Draw tooltip background
                painter.rect_filled(tooltip_rect, 4.0, Color32::from_rgb(40, 45, 55));
                painter.rect_stroke(tooltip_rect, 4.0, Stroke::new(1.0, Color32::from_rgb(80, 85, 95)));

                // Draw tooltip text
                painter.text(
                    Pos2::new(tooltip_rect.left() + 5.0, tooltip_rect.top() + 5.0),
                    egui::Align2::LEFT_TOP,
                    &node.full_label,
                    egui::FontId::proportional(10.0),
                    Color32::WHITE,
                );
                painter.text(
                    Pos2::new(tooltip_rect.left() + 5.0, tooltip_rect.top() + 18.0),
                    egui::Align2::LEFT_TOP,
                    format!("Messages: {}", node.message_count),
                    egui::FontId::proportional(9.0),
                    Color32::LIGHT_GRAY,
                );
                painter.text(
                    Pos2::new(tooltip_rect.left() + 5.0, tooltip_rect.top() + 30.0),
                    egui::Align2::LEFT_TOP,
                    format!("Sent: {} | Recv: {}", node.sent_count, node.received_count),
                    egui::FontId::proportional(9.0),
                    Color32::LIGHT_GRAY,
                );
            }
        }
    }

    // Stats in corner
    let stats_text = format!(
        "{} agents, {} msgs",
        state.data.stats.agent_count, state.data.stats.total_messages
    );
    painter.text(
        Pos2::new(rect.right() - 5.0, rect.bottom() - 5.0),
        egui::Align2::RIGHT_BOTTOM,
        stats_text,
        egui::FontId::proportional(9.0),
        Color32::from_rgb(100, 100, 100),
    );

    // Request repaint for animation
    ui.ctx().request_repaint();

    response
}

/// Calculate node radius based on message count.
fn node_radius(message_count: i32, data: &super::types::MailNetworkData) -> f32 {
    let max_count = data.nodes.iter().map(|n| n.message_count).max().unwrap_or(1);
    let min_radius = 6.0;
    let max_radius = 18.0;

    let normalized = (message_count as f32) / (max_count.max(1) as f32);
    min_radius + normalized.sqrt() * (max_radius - min_radius)
}

/// Draw an arrow head at the target end of an edge.
fn draw_arrow_head(painter: &Painter, from: Pos2, to: Pos2, color: Color32, thickness: f32) {
    let dir = (to - from).normalized();
    let arrow_len = 6.0 + thickness;
    let arrow_width = 3.0 + thickness * 0.5;

    // Position arrow head slightly before target (to not overlap node)
    let arrow_tip = to - dir * 8.0;
    let arrow_base = arrow_tip - dir * arrow_len;

    let perp = Vec2::new(-dir.y, dir.x);
    let left = arrow_base + perp * arrow_width;
    let right = arrow_base - perp * arrow_width;

    painter.add(egui::Shape::convex_polygon(
        vec![arrow_tip, left, right],
        color,
        Stroke::NONE,
    ));
}
