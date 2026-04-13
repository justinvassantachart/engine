use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::debug::dwarf::DieReference;

enum ScopeEntry {
    /// Root scope for a frame — returns all locals/params/globals
    Frame { frame_id: u32 },
    
    /// TODO: handle nested types somehow
    // Variable {
        // TODO
    // },
}

#[derive(Default)]
pub struct ScopeMap {
    next_ref: i64,
    entries: HashMap<i64, ScopeEntry>,
}

// ---------------------------------------------------------------------------
// Base Protocol
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ProtocolMessage {
    #[serde(rename = "request")]
    Request {
        seq: i64,
        command: String,
        #[serde(default)]
        arguments: Option<serde_json::Value>,
    },
    #[serde(rename = "response")]
    Response {
        seq: i64,
        request_seq: i64,
        success: bool,
        command: String,
        #[serde(default)]
        message: Option<String>,
        #[serde(default)]
        body: Option<serde_json::Value>,
    },
    #[serde(rename = "event")]
    Event {
        seq: i64,
        event: String,
        #[serde(default)]
        body: Option<serde_json::Value>,
    },
}
