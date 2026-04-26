use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::debug::Value;

/// Tracks variable handles handed out via DAP `variablesReference` IDs.
///
/// DAP requires that any non-zero `variablesReference` returned in a
/// `scopes`/`variables` response can be looked up later via a `variables`
/// request. We allocate fresh IDs on each pause so that stale handles from a
/// previous stop don't accidentally resolve.
#[derive(Default)]
pub struct VariablesMap {
    next_ref: i64,
    entries: HashMap<i64, Vec<Value>>,
}

impl VariablesMap {
    /// Stores `vars` and returns a fresh non-zero `variablesReference`.
    pub fn allocate(&mut self, vars: Vec<Value>) -> i64 {
        self.next_ref += 1;
        let id = self.next_ref;
        self.entries.insert(id, vars);
        id
    }

    /// Returns the variables previously registered under `reference`, if any.
    pub fn get(&self, reference: i64) -> Option<&Vec<Value>> {
        self.entries.get(&reference)
    }

    /// Drops all currently registered handles. Called when the program resumes
    /// so old references don't survive past their stop.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
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
