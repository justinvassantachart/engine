use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;

use crate::dap::types::ProtocolMessage;
use crate::debug::Debugger;
use crate::types::DebugInfo;

struct DapState {
    seq_counter: i64,
    debugger: Option<Debugger>,
    callback: Option<js_sys::Function>,
    _closure: Option<Closure<dyn FnMut(web_sys::MessageEvent)>>,
}

impl DapState {
    fn next_seq(&mut self) -> i64 {
        self.seq_counter += 1;
        self.seq_counter
    }

    fn debugger(&self) -> Option<&Debugger> {
        self.debugger.as_ref()
    }

    fn handle_set_breakpoints(
        &self,
        rseq: i64,
        seq: i64,
        command: &str,
        args: &serde_json::Value,
    ) -> ProtocolMessage {
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

        let Some(dbg) = self.debugger() else {
            return err(rseq, seq, command, "No debugger attached");
        };
        let results = dbg.set_breakpoints(source, &lines);
        let bps: Vec<_> = results
            .iter()
            .map(|(line, verified)| serde_json::json!({ "verified": verified, "line": line }))
            .collect();
        ok(rseq, seq, command, serde_json::json!({ "breakpoints": bps }))
    }

    fn handle_stack_trace(&self, rseq: i64, seq: i64, command: &str) -> ProtocolMessage {
        let Some(dbg) = self.debugger() else {
            return err(rseq, seq, command, "No debugger attached");
        };
        let frames = dbg.backtrace();
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
            command,
            serde_json::json!({
                "stackFrames": stack_frames,
                "totalFrames": total,
            }),
        )
    }

    fn handle_continue(&self, rseq: i64, seq: i64, command: &str) -> ProtocolMessage {
        if let Some(dbg) = self.debugger() {
            dbg.continue_();
        }
        ok(rseq, seq, command, serde_json::json!({}))
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
        let cb = s.callback.clone();
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
                callback: None,
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
            "setBreakpoints" => state.handle_set_breakpoints(rseq, seq, &command, &args),
            "stackTrace" => state.handle_stack_trace(rseq, seq, &command),
            "continue" => state.handle_continue(rseq, seq, &command),
            other => err(rseq, seq, other, &format!("Unknown command: {other}")),
        };

        serde_wasm_bindgen::to_value(&response).unwrap_or(JsValue::NULL)
    }

    /// Registers a callback that receives all DAP events.
    pub fn on(&self, callback: js_sys::Function) {
        self.state.borrow_mut().callback = Some(callback);
    }
}
