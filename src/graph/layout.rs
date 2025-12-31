//! Force-directed graph layout algorithm.
//!
//! Implements a simple force-directed layout with:
//! - Repulsion between all nodes (Coulomb's law)
//! - Attraction along edges (Hooke's law)
//! - Centering force toward graph center
//! - Damping to settle the simulation

use super::types::GraphState;
use egui::{Pos2, Vec2};

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
}

impl Default for ForceLayout {
    fn default() -> Self {
        Self {
            repulsion: 5000.0,
            attraction: 0.01,
            centering: 0.001,
            damping: 0.85,
            min_distance: 30.0,
            max_velocity: 50.0,
            ideal_length: 100.0,
        }
    }
}

impl ForceLayout {
    /// Run one iteration of the force simulation
    pub fn step(&self, state: &mut GraphState, center: Pos2) {
        if !state.physics_enabled || state.data.nodes.is_empty() {
            return;
        }

        let node_ids: Vec<String> = state.data.nodes.iter().map(|n| n.id.clone()).collect();

        // Calculate forces for each node
        let mut forces: Vec<Vec2> = vec![Vec2::ZERO; node_ids.len()];

        // Repulsion between all pairs of nodes
        for i in 0..node_ids.len() {
            let pos_i = match state.positions.get(&node_ids[i]) {
                Some(&p) => p,
                None => continue,
            };

            for j in (i + 1)..node_ids.len() {
                let pos_j = match state.positions.get(&node_ids[j]) {
                    Some(&p) => p,
                    None => continue,
                };

                let delta = pos_i - pos_j;
                let distance = delta.length().max(self.min_distance);
                let force_magnitude = self.repulsion / (distance * distance);

                let force = delta.normalized() * force_magnitude;
                forces[i] += force;
                forces[j] -= force;
            }
        }

        // Attraction along edges
        for edge in &state.data.edges {
            let source_idx = match state.node_index.get(&edge.source) {
                Some(&i) => i,
                None => continue,
            };
            let target_idx = match state.node_index.get(&edge.target) {
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
            let distance = delta.length().max(self.min_distance);
            let displacement = distance - self.ideal_length;
            let force_magnitude = self.attraction * displacement;

            let force = delta.normalized() * force_magnitude;
            forces[source_idx] += force;
            forces[target_idx] -= force;
        }

        // Centering force
        for (i, id) in node_ids.iter().enumerate() {
            if let Some(&pos) = state.positions.get(id) {
                let to_center = center - pos;
                forces[i] += to_center * self.centering;
            }
        }

        // Apply forces and update positions
        for (i, id) in node_ids.iter().enumerate() {
            // Update velocity
            if let Some(vel) = state.velocities.get_mut(id) {
                *vel = (*vel + forces[i]) * self.damping;

                // Clamp velocity
                if vel.length() > self.max_velocity {
                    *vel = vel.normalized() * self.max_velocity;
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
