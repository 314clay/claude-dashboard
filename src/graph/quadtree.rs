//! Barnes-Hut quadtree for O(n log n) force calculation.
//!
//! Instead of calculating repulsion between all pairs of nodes O(n²),
//! we group distant nodes and treat them as a single center of mass.

use egui::{Pos2, Vec2};

/// A node in the quadtree - either a leaf with one body, or an internal node with children
#[derive(Debug)]
pub enum QuadNode {
    Empty,
    Leaf {
        pos: Pos2,
        mass: f32,
    },
    Internal {
        /// Center of mass of all bodies in this cell
        center_of_mass: Pos2,
        /// Total mass of all bodies in this cell
        total_mass: f32,
        /// Number of bodies in this cell
        count: u32,
        /// Children: NW, NE, SW, SE
        children: Box<[QuadNode; 4]>,
    },
}

/// Axis-aligned bounding box for quadtree cells
#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    pub min: Pos2,
    pub max: Pos2,
}

impl Bounds {
    pub fn new(min: Pos2, max: Pos2) -> Self {
        Self { min, max }
    }

    pub fn center(&self) -> Pos2 {
        Pos2::new(
            (self.min.x + self.max.x) / 2.0,
            (self.min.y + self.max.y) / 2.0,
        )
    }

    pub fn size(&self) -> f32 {
        (self.max.x - self.min.x).max(self.max.y - self.min.y)
    }

    pub fn contains(&self, pos: Pos2) -> bool {
        pos.x >= self.min.x && pos.x <= self.max.x && pos.y >= self.min.y && pos.y <= self.max.y
    }

    /// Get the quadrant for a position (0=NW, 1=NE, 2=SW, 3=SE)
    pub fn quadrant(&self, pos: Pos2) -> usize {
        let center = self.center();
        let east = pos.x >= center.x;
        let south = pos.y >= center.y;
        match (south, east) {
            (false, false) => 0, // NW
            (false, true) => 1,  // NE
            (true, false) => 2,  // SW
            (true, true) => 3,   // SE
        }
    }

    /// Get bounds for a specific quadrant
    pub fn child_bounds(&self, quadrant: usize) -> Bounds {
        let center = self.center();
        match quadrant {
            0 => Bounds::new(self.min, center), // NW
            1 => Bounds::new(Pos2::new(center.x, self.min.y), Pos2::new(self.max.x, center.y)), // NE
            2 => Bounds::new(Pos2::new(self.min.x, center.y), Pos2::new(center.x, self.max.y)), // SW
            3 => Bounds::new(center, self.max), // SE
            _ => unreachable!(),
        }
    }
}

/// Barnes-Hut quadtree for efficient force calculation
pub struct Quadtree {
    pub root: QuadNode,
    pub bounds: Bounds,
    /// Theta parameter: cell_size / distance threshold for approximation
    /// Higher = faster but less accurate. 1.0 is good for visualization.
    pub theta: f32,
}

impl Quadtree {
    /// Build a quadtree from a set of positions and masses
    pub fn build(positions: &[(Pos2, f32)], theta: f32) -> Self {
        if positions.is_empty() {
            return Self {
                root: QuadNode::Empty,
                bounds: Bounds::new(Pos2::ZERO, Pos2::ZERO),
                theta,
            };
        }

        // Find bounding box with some padding
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;

        for (pos, _) in positions {
            min_x = min_x.min(pos.x);
            min_y = min_y.min(pos.y);
            max_x = max_x.max(pos.x);
            max_y = max_y.max(pos.y);
        }

        // Add padding and make square
        let padding = 100.0;
        min_x -= padding;
        min_y -= padding;
        max_x += padding;
        max_y += padding;

        // Make it square (required for proper quadtree)
        let size = (max_x - min_x).max(max_y - min_y);
        max_x = min_x + size;
        max_y = min_y + size;

        let bounds = Bounds::new(Pos2::new(min_x, min_y), Pos2::new(max_x, max_y));

        let mut tree = Self {
            root: QuadNode::Empty,
            bounds,
            theta,
        };

        for &(pos, mass) in positions {
            tree.insert(pos, mass);
        }

        tree
    }

    /// Insert a body into the quadtree
    pub fn insert(&mut self, pos: Pos2, mass: f32) {
        self.root = Self::insert_into(std::mem::take(&mut self.root), pos, mass, self.bounds, 0);
    }

    fn insert_into(node: QuadNode, pos: Pos2, mass: f32, bounds: Bounds, depth: u32) -> QuadNode {
        // Prevent infinite recursion for coincident points
        if depth > 50 {
            return node;
        }

        match node {
            QuadNode::Empty => QuadNode::Leaf { pos, mass },

            QuadNode::Leaf {
                pos: existing_pos,
                mass: existing_mass,
            } => {
                // Convert to internal node and insert both
                let mut children = Box::new([
                    QuadNode::Empty,
                    QuadNode::Empty,
                    QuadNode::Empty,
                    QuadNode::Empty,
                ]);

                // Insert existing body
                let eq = bounds.quadrant(existing_pos);
                children[eq] = Self::insert_into(
                    QuadNode::Empty,
                    existing_pos,
                    existing_mass,
                    bounds.child_bounds(eq),
                    depth + 1,
                );

                // Insert new body
                let nq = bounds.quadrant(pos);
                children[nq] = Self::insert_into(
                    std::mem::take(&mut children[nq]),
                    pos,
                    mass,
                    bounds.child_bounds(nq),
                    depth + 1,
                );

                // Calculate combined center of mass
                let total_mass = existing_mass + mass;
                let center_of_mass = Pos2::new(
                    (existing_pos.x * existing_mass + pos.x * mass) / total_mass,
                    (existing_pos.y * existing_mass + pos.y * mass) / total_mass,
                );

                QuadNode::Internal {
                    center_of_mass,
                    total_mass,
                    count: 2,
                    children,
                }
            }

            QuadNode::Internal {
                center_of_mass,
                total_mass,
                count,
                mut children,
            } => {
                // Insert into appropriate child
                let q = bounds.quadrant(pos);
                children[q] = Self::insert_into(
                    std::mem::take(&mut children[q]),
                    pos,
                    mass,
                    bounds.child_bounds(q),
                    depth + 1,
                );

                // Update center of mass
                let new_total = total_mass + mass;
                let new_com = Pos2::new(
                    (center_of_mass.x * total_mass + pos.x * mass) / new_total,
                    (center_of_mass.y * total_mass + pos.y * mass) / new_total,
                );

                QuadNode::Internal {
                    center_of_mass: new_com,
                    total_mass: new_total,
                    count: count + 1,
                    children,
                }
            }
        }
    }

    /// Calculate the repulsion force on a body at `pos` with `mass`
    /// using Barnes-Hut approximation.
    ///
    /// Returns the force vector.
    pub fn calculate_force(
        &self,
        pos: Pos2,
        repulsion: f32,
        min_distance: f32,
    ) -> Vec2 {
        self.calculate_force_recursive(&self.root, pos, repulsion, min_distance, self.bounds)
    }

    fn calculate_force_recursive(
        &self,
        node: &QuadNode,
        pos: Pos2,
        repulsion: f32,
        min_distance: f32,
        bounds: Bounds,
    ) -> Vec2 {
        match node {
            QuadNode::Empty => Vec2::ZERO,

            QuadNode::Leaf {
                pos: body_pos,
                mass: body_mass,
            } => {
                let delta = pos - *body_pos;
                let distance = delta.length().max(min_distance);

                // Skip self (distance ≈ 0)
                if distance < 0.01 {
                    return Vec2::ZERO;
                }

                // Coulomb repulsion: F = k * m / r²
                let force_magnitude = repulsion * body_mass / (distance * distance);
                // Safe: use delta/distance instead of normalized() since distance is clamped non-zero
                (delta / distance) * force_magnitude
            }

            QuadNode::Internal {
                center_of_mass,
                total_mass,
                children,
                ..
            } => {
                let delta = pos - *center_of_mass;
                let distance = delta.length().max(min_distance);

                // Barnes-Hut criterion: if cell is far enough, use approximation
                let cell_size = bounds.size();
                if cell_size / distance < self.theta {
                    // Treat entire cell as single body at center of mass
                    let force_magnitude = repulsion * total_mass / (distance * distance);
                    // Safe: use delta/distance instead of normalized() since distance is clamped non-zero
                    (delta / distance) * force_magnitude
                } else {
                    // Cell too close, recurse into children
                    let mut force = Vec2::ZERO;
                    for (i, child) in children.iter().enumerate() {
                        force += self.calculate_force_recursive(
                            child,
                            pos,
                            repulsion,
                            min_distance,
                            bounds.child_bounds(i),
                        );
                    }
                    force
                }
            }
        }
    }
}

impl Default for QuadNode {
    fn default() -> Self {
        QuadNode::Empty
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quadtree_build() {
        let positions = vec![
            (Pos2::new(0.0, 0.0), 1.0),
            (Pos2::new(100.0, 0.0), 1.0),
            (Pos2::new(0.0, 100.0), 1.0),
            (Pos2::new(100.0, 100.0), 1.0),
        ];

        let tree = Quadtree::build(&positions, 1.0);

        match &tree.root {
            QuadNode::Internal { count, .. } => assert_eq!(*count, 4),
            _ => panic!("Expected internal node"),
        }
    }

    #[test]
    fn test_force_calculation() {
        let positions = vec![
            (Pos2::new(0.0, 0.0), 1.0),
            (Pos2::new(100.0, 0.0), 1.0),
        ];

        let tree = Quadtree::build(&positions, 1.0);

        // Force on first body should push it left (away from second body)
        let force = tree.calculate_force(Pos2::new(0.0, 0.0), 1000.0, 1.0);
        assert!(force.x < 0.0, "Force should push left: {:?}", force);
    }
}
