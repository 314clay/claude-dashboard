//! Beads (issues) panel with lazy loading and virtual scrolling.
//!
//! Performance optimizations:
//! - Lazy loading: Only parse JSONL when panel is visible
//! - Caching: Cache parsed data with mtime-based invalidation
//! - Virtual scrolling: Only render visible items

pub mod loader;
pub mod widget;

pub use loader::BeadLoader;
pub use widget::render_beads_panel;
