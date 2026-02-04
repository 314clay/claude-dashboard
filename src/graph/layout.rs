//! Force-directed graph layout algorithm.
//!
//! Implements a simple force-directed layout with:
//! - Repulsion between all nodes (Coulomb's law) - O(n log n) via Barnes-Hut
//! - Attraction along edges (Hooke's law)
//! - Centering force toward graph center
//! - Damping to settle the simulation

use super::quadtree::Quadtree;
use super::types::{GraphState, Role};
use egui::{Pos2, Rect, Vec2};
use rand::seq::SliceRandom;
use std::collections::{HashMap, HashSet};

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
    /// How much visual size affects mass/charge (0 = uniform, higher = more differentiation)
    pub size_physics_weight: f32,
}

impl Default for ForceLayout {
    fn default() -> Self {
        Self {
            repulsion: 12000.0,      // Increased for better spread
            attraction: 0.08,         // Slightly reduced for looser clustering
            centering: 0.0002,        // Doubled to keep nodes more centered
            damping: 0.88,            // Slightly increased for faster settling
            min_distance: 30.0,
            max_velocity: 50.0,
            ideal_length: 120.0,      // Increased for more spacing
            temporal_strength: 0.5,
            size_physics_weight: 0.0,
        }
    }
}

impl ForceLayout {
    /// Run one iteration of the force simulation
    /// If `visible_nodes` is Some, only simulate those nodes (filtered view)
    /// `node_sizes` maps node IDs to their visual sizes (for mass-based physics)
    pub fn step(
        &self,
        state: &mut GraphState,
        center: Pos2,
        visible_nodes: Option<&HashSet<String>>,
        node_sizes: Option<&HashMap<String, f32>>,
    ) {
        if !state.physics_enabled || state.data.nodes.is_empty() {
            return;
        }

        // Filter to only visible nodes if filter is active
        let node_ids: Vec<String> = state
            .data
            .nodes
            .iter()
            .filter(|n| visible_nodes.map_or(true, |v| v.contains(&n.id)))
            .map(|n| n.id.clone())
            .collect();

        if node_ids.is_empty() {
            return;
        }

        // Build local index for filtered nodes (forces array indices)
        let local_index: HashMap<String, usize> = node_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), i))
            .collect();

        // Calculate forces for each visible node
        let mut forces: Vec<Vec2> = vec![Vec2::ZERO; node_ids.len()];

        // Compute node masses from sizes
        // mass = 1.0 + weight * normalized_size (range: 1.0 to 1.0 + weight)
        // At weight=0, all masses are 1.0 (uniform)
        let (min_size, max_size) = if let Some(sizes) = node_sizes {
            let min = sizes.values().cloned().fold(f32::MAX, f32::min);
            let max = sizes.values().cloned().fold(0.0_f32, f32::max);
            (min, max.max(min + 0.001)) // Avoid division by zero
        } else {
            (1.0, 1.0)
        };

        let node_masses: HashMap<String, f32> = node_ids
            .iter()
            .map(|id| {
                let size = node_sizes
                    .and_then(|s| s.get(id))
                    .copied()
                    .unwrap_or(1.0);
                let normalized = (size - min_size) / (max_size - min_size);
                let mass = 1.0 + self.size_physics_weight * normalized;
                (id.clone(), mass.max(0.1)) // Floor to prevent instability
            })
            .collect();

        // Repulsion using Barnes-Hut quadtree - O(n log n) instead of O(n²)
        // Only include visible nodes in the tree, with computed masses
        let positions_with_mass: Vec<(Pos2, f32)> = node_ids
            .iter()
            .filter_map(|id| {
                let pos = state.positions.get(id)?;
                let mass = node_masses.get(id).copied().unwrap_or(1.0);
                Some((*pos, mass))
            })
            .collect();

        let tree = Quadtree::build(&positions_with_mass, 1.0); // theta = 1.0

        for (i, id) in node_ids.iter().enumerate() {
            if let Some(&pos) = state.positions.get(id) {
                let repulsion_force = tree.calculate_force(pos, self.repulsion, self.min_distance);
                forces[i] += repulsion_force;
            }
        }

        // Separate temporal edges from regular edges for stochastic sampling
        // Only include edges where BOTH endpoints are visible
        let is_edge_visible = |e: &&super::types::GraphEdge| {
            visible_nodes.map_or(true, |v| v.contains(&e.source) && v.contains(&e.target))
        };

        let (temporal_edges, regular_edges): (Vec<_>, Vec<_>) = state
            .data
            .edges
            .iter()
            .filter(is_edge_visible)
            .partition(|e| e.is_temporal);

        // Process ALL regular edges (structural edges are important)
        for edge in &regular_edges {
            self.apply_edge_force(edge, state, &local_index, &mut forces, 1.0, &node_masses);
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
                self.apply_edge_force(edge, state, &local_index, &mut forces, scale, &node_masses);
            }
        }

        // Centering force
        for (i, id) in node_ids.iter().enumerate() {
            if let Some(&pos) = state.positions.get(id) {
                let to_center = center - pos;
                forces[i] += to_center * self.centering;
            }
        }

        // Apply forces and update positions (only for visible nodes)
        // F = ma, so a = F/m - lighter nodes accelerate more from the same force
        for (i, id) in node_ids.iter().enumerate() {
            // Update velocity (divide force by mass so light nodes move more)
            if let Some(vel) = state.velocities.get_mut(id) {
                let mass = node_masses.get(id).copied().unwrap_or(1.0);
                let acceleration = forces[i] / mass;
                *vel = (*vel + acceleration) * self.damping;

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
    /// If `visible_nodes` is Some, only check velocity of visible nodes
    pub fn is_settled(&self, state: &GraphState, visible_nodes: Option<&HashSet<String>>) -> bool {
        let (total_velocity, count): (f32, usize) = state
            .velocities
            .iter()
            .filter(|(id, _)| visible_nodes.map_or(true, |v| v.contains(*id)))
            .fold((0.0, 0), |(sum, cnt), (_, v)| (sum + v.length(), cnt + 1));
        let avg_velocity = total_velocity / count.max(1) as f32;
        avg_velocity < 0.5
    }

    /// Run one iteration of the timeline layout simulation.
    /// X positions are fixed by timestamp, Y positions use physics.
    /// User messages are pinned at y=0, other nodes spread above.
    pub fn step_timeline(
        &self,
        state: &mut GraphState,
        bounds: Rect,
        visible_nodes: Option<&HashSet<String>>,
        node_sizes: Option<&HashMap<String, f32>>,
    ) {
        if !state.physics_enabled || state.data.nodes.is_empty() {
            return;
        }

        // Get time range from timeline state
        let min_time = state.timeline.min_time;
        let max_time = state.timeline.max_time;
        let time_range = max_time - min_time;

        if time_range <= 0.0 {
            return; // No valid time range
        }

        // Filter to visible nodes
        let node_ids: Vec<String> = state
            .data
            .nodes
            .iter()
            .filter(|n| visible_nodes.map_or(true, |v| v.contains(&n.id)))
            .map(|n| n.id.clone())
            .collect();

        if node_ids.is_empty() {
            return;
        }

        // Build local index for filtered nodes
        let local_index: HashMap<String, usize> = node_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), i))
            .collect();

        // Calculate forces (Y-only)
        let mut forces: Vec<f32> = vec![0.0; node_ids.len()]; // Y-component only

        // Compute node masses from sizes
        let (min_size, max_size) = if let Some(sizes) = node_sizes {
            let min = sizes.values().cloned().fold(f32::MAX, f32::min);
            let max = sizes.values().cloned().fold(0.0_f32, f32::max);
            (min, max.max(min + 0.001))
        } else {
            (1.0, 1.0)
        };

        let node_masses: HashMap<String, f32> = node_ids
            .iter()
            .map(|id| {
                let size = node_sizes
                    .and_then(|s| s.get(id))
                    .copied()
                    .unwrap_or(1.0);
                let normalized = (size - min_size) / (max_size - min_size);
                let mass = 1.0 + self.size_physics_weight * normalized;
                (id.clone(), mass.max(0.1))
            })
            .collect();

        // First pass: Set X positions based on timestamps and initialize Y
        let graph_width = bounds.width() * 0.9; // Leave some padding
        let x_offset = bounds.min.x + bounds.width() * 0.05;
        let y_baseline = bounds.center().y; // User messages go here

        for (_i, id) in node_ids.iter().enumerate() {
            if let Some(&node_idx) = state.node_index.get(id) {
                let node = &state.data.nodes[node_idx];

                // Calculate X from timestamp
                if let Some(ts) = node.timestamp_secs() {
                    let normalized_time = ((ts - min_time) / time_range) as f32;
                    let x = x_offset + normalized_time * graph_width;

                    if let Some(pos) = state.positions.get_mut(id) {
                        pos.x = x;

                        // Pin user messages at y_baseline
                        if node.role == Role::User {
                            pos.y = y_baseline;
                        }
                    }
                }
            }
        }

        // Y-only repulsion: push nodes apart vertically
        for (i, id_i) in node_ids.iter().enumerate() {
            let pos_i = match state.positions.get(id_i) {
                Some(&p) => p,
                None => continue,
            };
            let mass_i = node_masses.get(id_i).copied().unwrap_or(1.0);

            // Check if this is a user node (pinned, doesn't receive forces)
            let is_user_i = state.node_index.get(id_i)
                .map(|&idx| state.data.nodes[idx].role == Role::User)
                .unwrap_or(false);

            if is_user_i {
                continue; // User nodes don't move
            }

            for (j, id_j) in node_ids.iter().enumerate() {
                if i >= j {
                    continue; // Only compute once per pair
                }

                let pos_j = match state.positions.get(id_j) {
                    Some(&p) => p,
                    None => continue,
                };

                // Compute Y distance (and small X influence for nearby nodes)
                let dx = pos_j.x - pos_i.x;
                let dy = pos_j.y - pos_i.y;

                // Only apply strong repulsion if nodes are close in X (within 50 pixels)
                let x_proximity = (-dx.abs() / 50.0).exp(); // 1.0 when same X, decays with distance

                let distance_y = dy.abs().max(self.min_distance);

                // Repulsion force (Y component only)
                let force_y = self.repulsion * x_proximity / (distance_y * distance_y);
                let direction = if dy > 0.0 { -1.0 } else { 1.0 };

                forces[i] += force_y * direction / mass_i;

                // Check if j is a user node
                let is_user_j = state.node_index.get(id_j)
                    .map(|&idx| state.data.nodes[idx].role == Role::User)
                    .unwrap_or(false);

                if !is_user_j {
                    let mass_j = node_masses.get(id_j).copied().unwrap_or(1.0);
                    forces[j] -= force_y * direction / mass_j;
                }
            }
        }

        // Edge attraction (Y-only) - pull connected nodes together vertically
        for edge in &state.data.edges {
            // Skip temporal edges in timeline view (they'd cramp everything)
            if edge.is_temporal {
                continue;
            }

            let source_idx = match local_index.get(&edge.source) {
                Some(&i) => i,
                None => continue,
            };
            let target_idx = match local_index.get(&edge.target) {
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

            let dy = pos_target.y - pos_source.y;
            let force_y = self.attraction * dy * 0.5; // Reduced attraction

            // Check roles
            let is_user_source = state.node_index.get(&edge.source)
                .map(|&idx| state.data.nodes[idx].role == Role::User)
                .unwrap_or(false);
            let is_user_target = state.node_index.get(&edge.target)
                .map(|&idx| state.data.nodes[idx].role == Role::User)
                .unwrap_or(false);

            if !is_user_source {
                forces[source_idx] += force_y;
            }
            if !is_user_target {
                forces[target_idx] -= force_y;
            }
        }

        // Gentle centering force toward baseline for non-user nodes
        // This keeps nodes from drifting too far
        for (i, id) in node_ids.iter().enumerate() {
            let is_user = state.node_index.get(id)
                .map(|&idx| state.data.nodes[idx].role == Role::User)
                .unwrap_or(false);

            if is_user {
                continue;
            }

            if let Some(&pos) = state.positions.get(id) {
                let dy_from_baseline = y_baseline - pos.y;
                forces[i] += dy_from_baseline * self.centering * 10.0; // Gentle pull toward baseline
            }
        }

        // Apply forces (Y-only) with damping
        for (i, id) in node_ids.iter().enumerate() {
            let is_user = state.node_index.get(id)
                .map(|&idx| state.data.nodes[idx].role == Role::User)
                .unwrap_or(false);

            if is_user {
                // Ensure user nodes stay at baseline
                if let Some(pos) = state.positions.get_mut(id) {
                    pos.y = y_baseline;
                }
                if let Some(vel) = state.velocities.get_mut(id) {
                    vel.y = 0.0;
                }
                continue;
            }

            if let Some(vel) = state.velocities.get_mut(id) {
                // Only update Y velocity
                vel.y = (vel.y + forces[i]) * self.damping;
                vel.x = 0.0; // No X velocity in timeline mode

                // Clamp Y velocity
                if vel.y.abs() > self.max_velocity {
                    vel.y = vel.y.signum() * self.max_velocity;
                }

                // Update Y position only
                if let Some(pos) = state.positions.get_mut(id) {
                    pos.y += vel.y;

                    // Keep nodes above baseline (user messages)
                    // Actually, let them spread both above and below for now
                    // We can constrain to above-only if desired
                }
            }
        }
    }

    /// Apply attraction force for a single edge
    /// Uses `node_index` to map node IDs to force array indices
    /// Edge force is scaled by geometric mean of node masses (small-small edges are weak)
    fn apply_edge_force(
        &self,
        edge: &super::types::GraphEdge,
        state: &GraphState,
        node_index: &HashMap<String, usize>,
        forces: &mut [Vec2],
        scale: f32,
        node_masses: &HashMap<String, f32>,
    ) {
        let source_idx = match node_index.get(&edge.source) {
            Some(&i) => i,
            None => return,
        };
        let target_idx = match node_index.get(&edge.target) {
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

        // Scale edge force by geometric mean of node masses
        // This makes small↔small edges weak, big↔big edges strong
        let mass_source = node_masses.get(&edge.source).copied().unwrap_or(1.0);
        let mass_target = node_masses.get(&edge.target).copied().unwrap_or(1.0);
        let mass_factor = (mass_source * mass_target).sqrt();

        let force_magnitude = self.attraction * displacement * edge_multiplier * mass_factor * scale;

        let force = delta.normalized() * force_magnitude;
        forces[source_idx] += force;
        forces[target_idx] -= force;
    }
}
