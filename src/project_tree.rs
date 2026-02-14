//! Collapsible project tree with tri-state checkboxes.
//!
//! Converts flat project paths like `~/Documents/GitHub/foo` into a tree
//! structure with shared-prefix grouping and single-child collapsing.

use std::collections::HashSet;

use egui::{Color32, Pos2, Rect, Response, Sense, Ui, Vec2};

use crate::theme;

/// Tri-state check value for tree nodes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CheckState {
    Checked,
    Unchecked,
    Mixed,
}

/// One node in the project tree.
#[derive(Debug, Clone)]
pub struct ProjectTreeNode {
    /// Display segment (e.g. `"GitHub"`)
    pub name: String,
    /// Full accumulated path (e.g. `"~/Documents/GitHub"`)
    pub full_path: String,
    pub children: Vec<ProjectTreeNode>,
    /// True when this node represents an actual project path.
    pub is_leaf: bool,
}

impl ProjectTreeNode {
    /// Build a tree from a sorted list of project path strings.
    pub fn build(projects: &[String]) -> ProjectTreeNode {
        let mut root = ProjectTreeNode {
            name: String::new(),
            full_path: String::new(),
            children: Vec::new(),
            is_leaf: false,
        };

        for path in projects {
            let segments: Vec<&str> = path.split('/').collect();
            insert(&mut root, &segments, 0);
        }

        // Merge single-child interior chains (e.g. `~/Documents/GitHub`)
        auto_collapse_single_children(&mut root);

        // Sort children alphabetically at every level
        sort_children(&mut root);

        root
    }

    /// Collect all leaf `full_path` values under this node.
    pub fn leaf_paths(&self) -> Vec<String> {
        let mut out = Vec::new();
        collect_leaves(self, &mut out);
        out
    }

    /// Determine the check state of this node given the set of selected projects.
    pub fn check_state(&self, selected: &HashSet<String>) -> CheckState {
        let leaves = self.leaf_paths();
        if leaves.is_empty() {
            return CheckState::Unchecked;
        }
        let count = leaves.iter().filter(|p| selected.contains(*p)).count();
        if count == 0 {
            CheckState::Unchecked
        } else if count == leaves.len() {
            CheckState::Checked
        } else {
            CheckState::Mixed
        }
    }
}

// ---------------------------------------------------------------------------
// Tree construction helpers
// ---------------------------------------------------------------------------

fn insert(node: &mut ProjectTreeNode, segments: &[&str], depth: usize) {
    if depth >= segments.len() {
        node.is_leaf = true;
        return;
    }

    let seg = segments[depth];

    // Find or create child
    let pos = node.children.iter().position(|c| c.name == seg);
    let idx = match pos {
        Some(i) => i,
        None => {
            let accumulated = if node.full_path.is_empty() {
                seg.to_string()
            } else {
                format!("{}/{}", node.full_path, seg)
            };
            node.children.push(ProjectTreeNode {
                name: seg.to_string(),
                full_path: accumulated,
                children: Vec::new(),
                is_leaf: false,
            });
            node.children.len() - 1
        }
    };

    insert(&mut node.children[idx], segments, depth + 1);
}

/// Merge single-child interior nodes into their parent.
fn auto_collapse_single_children(node: &mut ProjectTreeNode) {
    // Recurse first so merging happens bottom-up.
    for child in &mut node.children {
        auto_collapse_single_children(child);
    }

    // If this node has exactly one child and that child is not a leaf,
    // absorb the child's name, full_path, and children.
    while node.children.len() == 1 && !node.children[0].is_leaf {
        let child = node.children.remove(0);
        node.name = if node.name.is_empty() {
            child.name
        } else {
            format!("{}/{}", node.name, child.name)
        };
        node.full_path = child.full_path;
        node.children = child.children;
    }
}

fn sort_children(node: &mut ProjectTreeNode) {
    node.children.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    for child in &mut node.children {
        sort_children(child);
    }
}

fn collect_leaves(node: &ProjectTreeNode, out: &mut Vec<String>) {
    if node.is_leaf {
        out.push(node.full_path.clone());
    }
    for child in &node.children {
        collect_leaves(child, out);
    }
}

// ---------------------------------------------------------------------------
// Tri-state checkbox widget
// ---------------------------------------------------------------------------

/// Paint a custom tri-state checkbox and return `Some(true)` to select-all
/// or `Some(false)` to deselect-all on click.
pub fn tri_state_checkbox(ui: &mut Ui, state: CheckState) -> Option<bool> {
    let size = Vec2::splat(ui.spacing().icon_width);
    let (rect, response): (Rect, Response) = ui.allocate_exact_size(size, Sense::click());

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let visuals = ui.style().interact(&response);
        let rounding = 2.0;

        match state {
            CheckState::Checked => {
                // Filled green square with a white checkmark
                painter.rect_filled(rect, rounding, theme::accent::GREEN);
                // Checkmark path
                let cx = rect.center().x;
                let cy = rect.center().y;
                let s = rect.width() * 0.2;
                let points = vec![
                    Pos2::new(cx - s * 1.4, cy),
                    Pos2::new(cx - s * 0.3, cy + s * 1.2),
                    Pos2::new(cx + s * 1.6, cy - s * 1.0),
                ];
                painter.line_segment(
                    [points[0], points[1]],
                    egui::Stroke::new(1.8, Color32::WHITE),
                );
                painter.line_segment(
                    [points[1], points[2]],
                    egui::Stroke::new(1.8, Color32::WHITE),
                );
            }
            CheckState::Mixed => {
                // Orange filled square with a white dash
                painter.rect_filled(rect, rounding, theme::accent::ORANGE);
                let y = rect.center().y;
                let inset = rect.width() * 0.25;
                painter.line_segment(
                    [
                        Pos2::new(rect.left() + inset, y),
                        Pos2::new(rect.right() - inset, y),
                    ],
                    egui::Stroke::new(2.0, Color32::WHITE),
                );
            }
            CheckState::Unchecked => {
                // Empty outlined box
                painter.rect_stroke(rect, rounding, visuals.bg_stroke);
            }
        }
    }

    if response.clicked() {
        // Unchecked or Mixed → select all; Checked → deselect all
        Some(state != CheckState::Checked)
    } else {
        None
    }
}
