//! Types for mail network graph visualization.

use egui::Pos2;
use serde::Deserialize;
use std::collections::HashMap;

/// Node in the mail network (an agent).
#[derive(Debug, Clone, Deserialize)]
pub struct MailNode {
    pub id: String,
    pub label: String,
    pub full_label: String,
    pub message_count: i32,
    pub sent_count: i32,
    pub received_count: i32,
}

/// Edge in the mail network (messages between agents).
#[derive(Debug, Clone, Deserialize)]
pub struct MailEdge {
    pub source: String,
    pub target: String,
    pub weight: f32,
    pub message_count: i32,
}

/// Statistics about the mail network.
#[derive(Debug, Clone, Deserialize)]
pub struct MailStats {
    pub total_messages: i32,
    pub agent_count: i32,
    pub max_edge_count: i32,
}

/// Response from the mail network API.
#[derive(Debug, Clone, Deserialize)]
pub struct MailNetworkData {
    pub nodes: Vec<MailNode>,
    pub edges: Vec<MailEdge>,
    pub stats: MailStats,
}

/// State for the mail network graph (positions, velocities, etc).
pub struct MailNetworkState {
    pub data: MailNetworkData,
    pub positions: HashMap<String, Pos2>,
    pub velocities: HashMap<String, egui::Vec2>,
    pub hovered_node: Option<String>,
    pub dragged_node: Option<String>,
    pub drag_offset: egui::Vec2,
}

impl MailNetworkState {
    pub fn new(data: MailNetworkData, center: Pos2, radius: f32) -> Self {
        let mut positions = HashMap::new();
        let mut velocities = HashMap::new();

        // Initialize nodes in a circle
        let n = data.nodes.len();
        for (i, node) in data.nodes.iter().enumerate() {
            let angle = 2.0 * std::f32::consts::PI * (i as f32) / (n.max(1) as f32);
            let x = center.x + radius * angle.cos();
            let y = center.y + radius * angle.sin();
            positions.insert(node.id.clone(), Pos2::new(x, y));
            velocities.insert(node.id.clone(), egui::Vec2::ZERO);
        }

        Self {
            data,
            positions,
            velocities,
            hovered_node: None,
            dragged_node: None,
            drag_offset: egui::Vec2::ZERO,
        }
    }

    /// Apply one step of force-directed layout.
    pub fn step(&mut self, center: Pos2, bounds: egui::Rect, dt: f32) {
        let repulsion = 5000.0;
        let attraction = 0.05;
        let damping = 0.85;

        // Skip if no nodes
        if self.data.nodes.is_empty() {
            return;
        }

        // Build node index
        let node_ids: Vec<&str> = self.data.nodes.iter().map(|n| n.id.as_str()).collect();

        // Compute forces
        let mut forces: HashMap<&str, egui::Vec2> = HashMap::new();
        for id in &node_ids {
            forces.insert(*id, egui::Vec2::ZERO);
        }

        // Repulsion between all pairs
        for i in 0..node_ids.len() {
            for j in (i + 1)..node_ids.len() {
                let id_a = node_ids[i];
                let id_b = node_ids[j];

                let pos_a = self.positions.get(id_a).copied().unwrap_or(center);
                let pos_b = self.positions.get(id_b).copied().unwrap_or(center);

                let diff = pos_a - pos_b;
                let dist_sq = diff.length_sq().max(100.0);
                let force_mag = repulsion / dist_sq;
                let force = diff.normalized() * force_mag;

                *forces.get_mut(id_a).unwrap() += force;
                *forces.get_mut(id_b).unwrap() -= force;
            }
        }

        // Attraction along edges
        for edge in &self.data.edges {
            let pos_src = self.positions.get(&edge.source).copied().unwrap_or(center);
            let pos_tgt = self.positions.get(&edge.target).copied().unwrap_or(center);

            let diff = pos_tgt - pos_src;
            let dist = diff.length().max(1.0);

            // Stronger attraction for edges with more messages
            let strength = attraction * (1.0 + edge.weight);
            let force = diff.normalized() * dist * strength;

            if let Some(f) = forces.get_mut(edge.source.as_str()) {
                *f += force;
            }
            if let Some(f) = forces.get_mut(edge.target.as_str()) {
                *f -= force;
            }
        }

        // Centering force
        for id in &node_ids {
            let pos = self.positions.get(*id).copied().unwrap_or(center);
            let to_center = center - pos;
            if let Some(f) = forces.get_mut(*id) {
                *f += to_center * 0.01;
            }
        }

        // Apply forces (skip dragged node)
        for id in &node_ids {
            if Some(id.to_string()) == self.dragged_node {
                continue;
            }

            let force = forces.get(*id).copied().unwrap_or(egui::Vec2::ZERO);
            let vel = self.velocities.get_mut(*id).unwrap();
            *vel = (*vel + force * dt) * damping;

            let pos = self.positions.get_mut(*id).unwrap();
            *pos += *vel * dt;

            // Constrain to bounds
            pos.x = pos.x.clamp(bounds.left() + 10.0, bounds.right() - 10.0);
            pos.y = pos.y.clamp(bounds.top() + 10.0, bounds.bottom() - 10.0);
        }
    }
}
