//! Force-directed graph layout algorithm.
//!
//! Implements a force-directed layout with:
//! - Repulsion between all nodes (Coulomb's law) using Barnes-Hut O(n log n)
//! - Attraction along edges (Hooke's law)
//! - Centering force toward graph center
//! - Damping to settle the simulation

use super::quadtree::Quadtree;
use super::types::GraphState;
use egui::{Pos2, Vec2};
use std::collections::HashSet;

/// Force-directed layout parameters
pub struct ForceLayout {
    /// Repulsion strength between nodes
    pub repulsion: f32,
    /// Attraction strength along edges
    pub attraction: f32,
    /// Centering force strength
    pub centering: f32,
    /// Damping factor (0.0 - 1.0)
    pub damping: f32,
    /// Minimum distance to prevent division by zero
    pub min_distance: f32,
    /// Maximum velocity
    pub max_velocity: f32,
    /// Ideal edge length
    pub ideal_length: f32,
    /// How much node size affects repulsion (0.0 = uniform, 1.0 = fully size-weighted)
    pub size_repulsion_weight: f32,
    /// Temporal edge attraction multiplier (scales the edge's similarity strength)
    pub temporal_strength: f32,
    /// Barnes-Hut theta parameter (higher = faster but less accurate, 1.0 recommended)
    pub theta: f32,
}

impl Default for ForceLayout {
    fn default() -> Self {
        Self {
            repulsion: 10000.0,
            attraction: 0.1,
            centering: 0.0001,
            damping: 0.85,
            min_distance: 30.0,
            max_velocity: 50.0,
            ideal_length: 100.0,
            size_repulsion_weight: 0.0, // Default: uniform repulsion
            temporal_strength: 0.5,     // Default: moderate temporal attraction
            theta: 1.0,                 // Barnes-Hut: 1.0 is good for visualization
        }
    }
}

impl ForceLayout {
    /// Run one iteration of the force simulation
    /// If `visible_nodes` is provided, only those nodes participate in physics.
    pub fn step(&self, state: &mut GraphState, center: Pos2, visible_nodes: Option<&HashSet<String>>) {
        if !state.physics_enabled || state.data.nodes.is_empty() {
            return;
        }

        // Collect only visible nodes for physics (or all if no filter)
        let node_ids: Vec<String> = state.data.nodes.iter()
            .filter(|n| visible_nodes.map_or(true, |v| v.contains(&n.id)))
            .map(|n| n.id.clone())
            .collect();

        if node_ids.is_empty() {
            return;
        }

        // Calculate forces for each node
        let mut forces: Vec<Vec2> = vec![Vec2::ZERO; node_ids.len()];

        // Build index map from node_id to position in our filtered list
        let id_to_force_idx: std::collections::HashMap<&str, usize> = node_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i))
            .collect();

        // Build importance lookup for size-weighted repulsion (used as mass in Barnes-Hut)
        let importances: Vec<f32> = node_ids.iter()
            .map(|id| {
                state.node_index.get(id)
                    .and_then(|&idx| state.data.nodes.get(idx))
                    .and_then(|n| n.importance_score)
                    .unwrap_or(0.5)
            })
            .collect();

        // Build positions with masses for quadtree
        let positions_with_mass: Vec<(Pos2, f32)> = node_ids.iter()
            .enumerate()
            .filter_map(|(i, id)| {
                state.positions.get(id).map(|&pos| {
                    // Mass based on importance (size-weighted repulsion)
                    let exponent = 1.0 + 3.0 * self.size_repulsion_weight;
                    let mass = importances[i].powf(exponent);
                    (pos, mass)
                })
            })
            .collect();

        // Build Barnes-Hut quadtree - O(n log n)
        let quadtree = Quadtree::build(&positions_with_mass, self.theta);

        // Calculate repulsion using Barnes-Hut - O(n log n) instead of O(n²)
        for (i, id) in node_ids.iter().enumerate() {
            if let Some(&pos) = state.positions.get(id) {
                let force = quadtree.calculate_force(pos, self.repulsion, self.min_distance);
                forces[i] += force;
            }
        }

        // Attraction along edges (only if both endpoints are visible)
        for edge in &state.data.edges {
            // Skip edges where either endpoint is hidden
            let source_force_idx = match id_to_force_idx.get(edge.source.as_str()) {
                Some(&i) => i,
                None => continue,
            };
            let target_force_idx = match id_to_force_idx.get(edge.target.as_str()) {
                Some(&i) => i,
                None => continue,
            };

            let pos_source = match state.positions.get(&edge.source) {
                Some(&p) => p,
                None => continue,
            };
            let pos_target = match state.positions.get(&edge.target) {
                Some(&p) => p,
                None => continue,
            };

            let delta = pos_target - pos_source;
            let distance = delta.length();

            // Skip if nodes are at same position (prevents NaN from normalized())
            if distance < self.min_distance {
                continue;
            }

            let displacement = distance - self.ideal_length;

            // Base attraction, modified by edge strength for temporal/similarity edges
            let edge_multiplier = if edge.is_temporal {
                // Temporal edges: use pre-computed similarity * temporal_strength
                edge.similarity.unwrap_or(1.0) * self.temporal_strength
            } else if edge.is_similarity {
                // Similarity edges also use their strength
                edge.similarity.unwrap_or(1.0)
            } else {
                // Regular edges: full strength
                1.0
            };

            let force_magnitude = self.attraction * displacement * edge_multiplier;

            // Safe normalization: delta / distance (we already checked distance >= min_distance)
            let force = (delta / distance) * force_magnitude;
            forces[source_force_idx] += force;
            forces[target_force_idx] -= force;
        }

        // Centering force (only for visible nodes)
        for (i, id) in node_ids.iter().enumerate() {
            if let Some(&pos) = state.positions.get(id) {
                let to_center = center - pos;
                forces[i] += to_center * self.centering;
            }
        }

        // Apply forces and update positions (only for visible nodes)
        for (i, id) in node_ids.iter().enumerate() {
            // Update velocity
            if let Some(vel) = state.velocities.get_mut(id) {
                *vel = (*vel + forces[i]) * self.damping;

                // Clamp velocity (safe: avoid normalized() on zero-length vector)
                let vel_len = vel.length();
                if vel_len > self.max_velocity {
                    *vel = (*vel / vel_len) * self.max_velocity;
                }

                // Update position
                if let Some(pos) = state.positions.get_mut(id) {
                    *pos += *vel;
                }
            }
        }
    }

    /// Check if the simulation has settled
    pub fn is_settled(&self, state: &GraphState) -> bool {
        let total_velocity: f32 = state.velocities.values().map(|v| v.length()).sum();
        let avg_velocity = total_velocity / state.data.nodes.len().max(1) as f32;
        avg_velocity < 0.5
    }
}
