use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;

use crate::dap::types::ProtocolMessage;
use crate::debug::Debugger;
use crate::types::DebugInfo;

struct DapState {
    seq_counter: i64,
    debugger: Option<Debugger>,
    event_callbacks: HashMap<String, js_sys::Function>,
    _closure: Option<Closure<dyn FnMut(web_sys::MessageEvent)>>,
}

impl DapState {
    fn next_seq(&mut self) -> i64 {
        self.seq_counter += 1;
        self.seq_counter
    }
}

fn ok(seq: i64, request_seq: i64, command: &str, body: serde_json::Value) -> ProtocolMessage {
    ProtocolMessage::Response {
        seq,
        request_seq,
        success: true,
        command: command.to_string(),
        message: None,
        body: Some(body),
    }
}

fn err(seq: i64, request_seq: i64, command: &str, msg: &str) -> ProtocolMessage {
    ProtocolMessage::Response {
        seq,
        request_seq,
        success: false,
        command: command.to_string(),
        message: Some(msg.to_string()),
        body: None,
    }
}

/// Emits a DAP event to the registered callback (if any).
/// Borrows state briefly to read the callback, then drops the borrow before
/// invoking the callback so the callback can safely call back into the adapter.
fn emit_event(state: &Rc<RefCell<DapState>>, event_name: &str, body: Option<serde_json::Value>) {
    let (callback, seq) = {
        let mut s = state.borrow_mut();
        let cb = s.event_callbacks.get(event_name).cloned();
        let seq = s.next_seq();
        (cb, seq)
    };

    if let Some(callback) = callback {
        let msg = ProtocolMessage::Event {
            seq,
            event: event_name.to_string(),
            body,
        };
        if let Ok(val) = serde_wasm_bindgen::to_value(&msg) {
            let _ = callback.call1(&JsValue::NULL, &val);
        }
    }
}

#[wasm_bindgen]
pub struct DapAdapter {
    state: Rc<RefCell<DapState>>,
}

#[wasm_bindgen]
impl DapAdapter {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(DapState {
                seq_counter: 0,
                debugger: None,
                event_callbacks: HashMap::new(),
                _closure: None,
            })),
        }
    }

    /// Attaches a worker to the adapter.
    ///
    /// Listens for the worker's `debug` message (containing `DebugInfo`)
    /// to construct the internal `Debugger`, and for `breakpoint` messages
    /// to emit DAP `stopped` events.
    pub fn attach(&self, worker: web_sys::Worker) {
        let state = self.state.clone();

        let closure = Closure::wrap(Box::new(move |event: web_sys::MessageEvent| {
            let data = event.data();
            let msg_type = js_sys::Reflect::get(&data, &"type".into())
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default();

            match msg_type.as_str() {
                "debug" => {
                    let info_val = js_sys::Reflect::get(&data, &"info".into())
                        .expect("debug message has info field");
                    let info: DebugInfo = serde_wasm_bindgen::from_value(info_val)
                        .expect("DebugInfo deserialization");

                    state.borrow_mut().debugger = Some(Debugger::new(info));
                    emit_event(&state, "initialized", None);
                }
                "breakpoint" => {
                    emit_event(
                        &state,
                        "stopped",
                        Some(serde_json::json!({ "reason": "breakpoint" })),
                    );
                }
                _ => {}
            }
        }) as Box<dyn FnMut(web_sys::MessageEvent)>);

        worker
            .add_event_listener_with_callback("message", closure.as_ref().unchecked_ref())
            .expect("Added message listener to worker");

        self.state.borrow_mut()._closure = Some(closure);
    }

    /// Sends a DAP request and returns the response.
    #[wasm_bindgen(js_name = "sendMessage")]
    pub fn send_message(&self, msg: JsValue) -> JsValue {
        let request: ProtocolMessage = match serde_wasm_bindgen::from_value(msg) {
            Ok(r) => r,
            Err(_) => return JsValue::NULL,
        };

        let ProtocolMessage::Request {
            seq,
            command,
            arguments,
        } = request
        else {
            return JsValue::NULL;
        };

        let args = arguments.unwrap_or(serde_json::Value::Null);
        let mut state = self.state.borrow_mut();
        let rseq = state.next_seq();

        let response = match command.as_str() {
            "setBreakpoints" => {
                let source = args
                    .get("source")
                    .and_then(|s| s.get("path"))
                    .and_then(|p| p.as_str())
                    .unwrap_or("");
                let lines: Vec<i64> = args
                    .get("breakpoints")
                    .and_then(|b| b.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|bp| bp.get("line").and_then(|l| l.as_i64()))
                            .collect()
                    })
                    .unwrap_or_default();

                match state
                    .debugger
                    .as_ref()
                    .map(|d| d.set_breakpoints(source, &lines))
                {
                    Some(results) => {
                        let bps: Vec<_> = results
                            .iter()
                            .map(|(line, verified)| {
                                serde_json::json!({ "verified": verified, "line": line })
                            })
                            .collect();
                        ok(
                            rseq,
                            seq,
                            &command,
                            serde_json::json!({ "breakpoints": bps }),
                        )
                    }
                    None => err(rseq, seq, &command, "No debugger attached"),
                }
            }

            "stackTrace" => match state.debugger.as_ref().map(|d| d.backtrace()) {
                Some(frames) => {
                    let total = frames.len();
                    let stack_frames: Vec<_> = frames
                        .iter()
                        .map(|f| {
                            serde_json::json!({
                                "id": f.id,
                                "name": f.name,
                                "line": f.line,
                                "column": f.column,
                            })
                        })
                        .collect();
                    ok(
                        rseq,
                        seq,
                        &command,
                        serde_json::json!({
                            "stackFrames": stack_frames,
                            "totalFrames": total,
                        }),
                    )
                }
                None => err(rseq, seq, &command, "No debugger attached"),
            },

            "continue" => {
                if let Some(dbg) = state.debugger.as_ref() {
                    dbg.continue_();
                }
                ok(rseq, seq, &command, serde_json::json!({}))
            }

            _ => err(rseq, seq, &command, &format!("Unknown command: {command}")),
        };

        serde_wasm_bindgen::to_value(&response).unwrap_or(JsValue::NULL)
    }

    /// Registers a callback for a DAP event type (`"initialized"`, `"stopped"`).
    pub fn on(&self, event: &str, callback: js_sys::Function) {
        self.state
            .borrow_mut()
            .event_callbacks
            .insert(event.to_string(), callback);
    }
}
