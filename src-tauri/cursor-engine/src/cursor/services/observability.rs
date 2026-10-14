//! Request traces. AgentHarbor's router already records every model call, so
//! the Cursor side keeps the recorder's shape and records nothing.
use bytes::Bytes;

use crate::store::{BlobId, Store};

#[derive(Clone, Default)]
pub struct CursorTraceService;

impl CursorTraceService {
    pub fn new(_store: Store) -> Self {
        Self
    }

    pub fn recorder(&self, request_id: &str) -> CursorTraceRecorder {
        CursorTraceRecorder { request_id: request_id.into() }
    }
}

#[derive(Clone)]
pub struct CursorTraceRecorder {
    request_id: std::sync::Arc<str>,
}

impl CursorTraceRecorder {
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn begin(&self, _conversation_id: Option<&str>, _route: &str, _model_id: Option<&str>) {}

    pub fn resume(&self) {}

    pub fn request(&self, _artifact_type: &str, _data: Bytes, _metadata: serde_json::Value) {}

    pub fn artifact(&self, _artifact_type: &str, _source: &str, _data: &[u8], _metadata: serde_json::Value) {}

    pub fn linked_blob(&self, _artifact_type: &str, _source: &str, _blob_id: &BlobId, _metadata: serde_json::Value) {}

    pub fn response_started(&self, _status: u16) {}

    pub fn response_chunk(&self, _source: &str, _data: Bytes) {}

    pub fn finish(&self, _error: Option<&str>) {}
}
