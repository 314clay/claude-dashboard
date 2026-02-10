//! API client module for communicating with the Python backend.

mod client;

pub use client::{ApiClient, EmbeddingGenResult, EmbeddingStats, FilterStatusResponse, ImportanceStats, IngestResult, ProximityEdgesResponse, RescoreEvent, RescoreProgress, RescoreResult};
