//! Mail network graph visualization.
//!
//! Shows a mini force-directed graph of Gas Town agent communication.
//! Nodes represent agents (polecats, witness, mayor, etc.).
//! Edges represent message counts between agents.

pub mod types;
pub mod widget;

pub use types::{MailNetworkData, MailNetworkState};
pub use widget::render_mail_network;
