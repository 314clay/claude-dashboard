//! Force-directed graph layout algorithm.
//!
//! Implements a simple force-directed layout with:
//! - Repulsion between all nodes (Coulomb's law) - O(n log n) via Barnes-Hut
//! - Attraction along edges (Hooke's law)
//! - Centering force toward graph center
//! - Damping to settle the simulation

use super::quadtree::Quadtree;
use super::types::GraphState;
use egui::{Pos2, Vec2};
use rand::seq::SliceRandom;

/// Maximum temporal edges to process per physics frame (stochastic sampling)
const TEMPORAL_EDGES_PER_FRAME: usize = 2000;

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
    /// Temporal edge strength multiplier
    pub temporal_strength: f32,
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
            temporal_strength: 0.5,
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

        // Repulsion using Barnes-Hut quadtree - O(n log n) instead of O(n²)
        let positions_with_mass: Vec<(Pos2, f32)> = node_ids
            .iter()
            .filter_map(|id| state.positions.get(id).map(|&pos| (pos, 1.0)))
            .collect();

        let tree = Quadtree::build(&positions_with_mass, 1.0); // theta = 1.0

        for (i, id) in node_ids.iter().enumerate() {
            if let Some(&pos) = state.positions.get(id) {
                let repulsion_force = tree.calculate_force(pos, self.repulsion, self.min_distance);
                forces[i] += repulsion_force;
            }
        }

        // Separate temporal edges from regular edges for stochastic sampling
        let (temporal_edges, regular_edges): (Vec<_>, Vec<_>) = state
            .data
            .edges
            .iter()
            .partition(|e| e.is_temporal);

        // Process ALL regular edges (structural edges are important)
        for edge in &regular_edges {
            self.apply_edge_force(edge, state, &mut forces, 1.0);
        }

        // Stochastic sampling: process a random subset of temporal edges
        // Scale force by sampling ratio to maintain correct average force
        let temporal_count = temporal_edges.len();
        if temporal_count > 0 {
            let sample_size = temporal_count.min(TEMPORAL_EDGES_PER_FRAME);
            let scale = temporal_count as f32 / sample_size as f32;

            // Sample without replacement
            let mut rng = rand::thread_rng();
            let sampled: Vec<_> = temporal_edges
                .choose_multiple(&mut rng, sample_size)
                .collect();

            for edge in sampled {
                self.apply_edge_force(edge, state, &mut forces, scale);
            }
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

    /// Apply attraction force for a single edge
    fn apply_edge_force(
        &self,
        edge: &super::types::GraphEdge,
        state: &GraphState,
        forces: &mut [Vec2],
        scale: f32,
    ) {
        let source_idx = match state.node_index.get(&edge.source) {
            Some(&i) => i,
            None => return,
        };
        let target_idx = match state.node_index.get(&edge.target) {
            Some(&i) => i,
            None => return,
        };

        let pos_source = match state.positions.get(&edge.source) {
            Some(&p) => p,
            None => return,
        };
        let pos_target = match state.positions.get(&edge.target) {
            Some(&p) => p,
            None => return,
        };

        let delta = pos_target - pos_source;
        let distance = delta.length().max(self.min_distance);
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

        let force_magnitude = self.attraction * displacement * edge_multiplier * scale;

        let force = delta.normalized() * force_magnitude;
        forces[source_idx] += force;
        forces[target_idx] -= force;
    }
}
