//! API client module for communicating with the Python backend.

mod client;

pub use client::{ApiClient, EmbeddingGenResult, EmbeddingStats, ImportanceStats, IngestResult, ProximityEdgesResponse, RescoreEvent, RescoreProgress, RescoreResult};
