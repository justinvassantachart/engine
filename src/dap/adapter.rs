use std::cell::RefCell;
use std::rc::Rc;

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{Value, json};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;

use crate::dap::types::{ProtocolMessage, VariablesMap};
use crate::debug::Debugger;
use crate::types::{DebugInfo, PauseReason};

struct DapState {
    seq_counter: i64,
    debugger: Option<Debugger>,
    /// `initialize` request was handled and the client received the capabilities response.
    client_initialized: bool,
    /// We emitted `initialized` for this debug session (once per worker / run).
    initialized_emitted: bool,
    callback: Option<js_sys::Function>,
    vars: VariablesMap,
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

    // Returns the capababilities. TODO: check what more we can support.
    fn handle_initialize(&self) -> Result<Value> {
        Ok(json!({
            "supportsConfigurationDoneRequest": true,
            "supportsStepBack": false,
            "supportsFunctionBreakpoints": false,
        }))
    }

    // interacts with wait_for_resume in the worker to unblock after config is done.
    fn handle_configuration_done(&mut self) -> Result<Value> {
        self.vars.clear();
        self.debugger()
            .context("configurationDone: debugger not ready")?
            .continue_();
        Ok(Value::Null)
    }

    // We do not support exception breakpoints, but handle the request so we do not blow up.
    fn handle_set_exception_breakpoints(&self) -> Result<Value> {
        Ok(json!({ "breakpoints": [] }))
    }

    // WASM doesn't have threads in the traditional sense, so we report a single "main" thread. NOTE: we might extend later.
    fn handle_threads(&self) -> Result<Value> {
        Ok(json!({
            "threads": [{ "id": 1, "name": "main" }]
        }))
    }

    fn handle_set_breakpoints(&self, args: &Value) -> Result<Value> {
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

        let dbg = self.debugger().context("No debugger attached")?;
        // TODO: set_breakpoints. once implemented, verify this handler does the right thing with the results (e.g. line numbers, verified status)
        let bps: Vec<_> = lines
            .iter()
            .map(|line| {
                let location = dbg.set_breakpoint(source, *line);
                if let Some(location) = location {
                    json!({ "verified": true, "line": location.line })
                } else {
                    json!({ "verified": false })
                }
            })
            .collect();
        Ok(json!({ "breakpoints": bps }))
    }

    fn handle_stack_trace(&self) -> Result<Value> {
        let dbg = self.debugger().context("No debugger attached")?;
        let frames = dbg.backtrace().context("Failed to get backtrace")?;
        let total = frames.len();
        let stack_frames: Vec<_> = frames
            .iter()
            .map(|f| {
                let source = f.source.as_ref().map(|p| json!({ "path": p }));
                json!({
                    "id": f.id,
                    "name": f.name,
                    "line": f.line,
                    "column": f.column,
                    // TODO: resolve later with jacob using DIE info from DWARF.
                    "source": source,
                })
            })
            .collect();
        Ok(json!({
            "stackFrames": stack_frames,
            "totalFrames": total,
        }))
    }

    fn handle_scopes(&mut self, args: &Value) -> Result<Value> {
        let frame_id = args.get("frameId").and_then(|v| v.as_i64()).unwrap_or(0) as u32;
        let dbg = self.debugger().context("No debugger attached")?;
        let (arguments, locals) = dbg.get_variables(frame_id);

        let mut scopes: Vec<Value> = Vec::new();
        if !arguments.is_empty() {
            let reference = self.vars.allocate(arguments);
            scopes.push(json!({
                "name": "Arguments",
                "variablesReference": reference,
                "expensive": false,
            }));
        }
        if !locals.is_empty() {
            let reference = self.vars.allocate(locals);
            scopes.push(json!({
                "name": "Locals",
                "variablesReference": reference,
                "expensive": false,
            }));
        }
        Ok(json!({ "scopes": scopes }))
    }

    fn handle_variables(&mut self, args: &Value) -> Result<Value> {
        let reference = args
            .get("variablesReference")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        // Snapshot the entries (and a borrow-free DebugInfo handle) before we
        // mutate `self.vars` to allocate sub-references.
        let entries = self
            .vars
            .get(reference)
            .context("Unknown variablesReference")?
            .clone();
        let dbg = self.debugger().context("No debugger attached")?;
        let info = dbg.info().clone();
        // Children must be precomputed: dispatching through `dbg` borrows `self`
        // immutably, which conflicts with `self.vars.allocate` in the loop.
        let children_per: Vec<_> = entries.iter().map(|var| dbg.children(var)).collect();

        let mut variables: Vec<Value> = Vec::with_capacity(entries.len());
        for (var, children) in entries.iter().zip(children_per) {
            let display = var.display(&info);
            let type_name = var.type_name();
            let sub_ref = if children.is_empty() {
                0
            } else {
                self.vars.allocate(children)
            };

            let mut v = json!({
                "name": var.name(),
                "value": display,
                "type": type_name,
                "variablesReference": sub_ref,
            });

            if let Some(map) = v.as_object_mut() {
                if let Some(addr) = var.address() {
                    map.insert("memoryReference".into(), addr.to_string().into());
                }
                if let Some(bs) = var.ty().byte_size() {
                    map.insert("presentationHint".into(), json!({ "byteSize": bs }));
                }
            }

            variables.push(v);
        }
        Ok(json!({ "variables": variables }))
    }

    fn handle_continue(&mut self) -> Result<Value> {
        self.vars.clear();
        if let Some(dbg) = self.debugger() {
            dbg.continue_();
        }
        Ok(json!({ "allThreadsContinued": true }))
    }

    fn handle_next(&mut self) -> Result<Value> {
        self.vars.clear();
        if let Some(dbg) = self.debugger() {
            dbg.step_over();
        }
        Ok(json!({}))
    }

    fn handle_step_in(&mut self) -> Result<Value> {
        self.vars.clear();
        if let Some(dbg) = self.debugger() {
            dbg.step_into();
        }
        Ok(json!({}))
    }

    fn handle_step_out(&mut self) -> Result<Value> {
        self.vars.clear();
        if let Some(dbg) = self.debugger() {
            dbg.step_out();
        }
        Ok(json!({}))
    }
}

fn respond(rseq: i64, seq: i64, command: &str, result: Result<Value>) -> ProtocolMessage {
    match result {
        Ok(body) => ProtocolMessage::Response {
            seq: rseq,
            request_seq: seq,
            success: true,
            command: command.to_string(),
            message: None,
            // DAP spec requires body to be omitted if null, so we convert it to an Option here.
            body: if body.is_null() { None } else { Some(body) },
        },
        Err(e) => ProtocolMessage::Response {
            seq: rseq,
            request_seq: seq,
            success: false,
            command: command.to_string(),
            message: Some(e.to_string()),
            body: None,
        },
    }
}

/// Emits a DAP event to the registered callback (if any).
/// Borrows state briefly to read the callback, then drops the borrow before
/// invoking the callback so the callback can safely call back into the adapter.
fn emit_event(state: &Rc<RefCell<DapState>>, event_name: &str, body: Option<Value>) {
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
        let ser = serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true);
        if let Ok(val) = msg.serialize(&ser) {
            let _ = callback.call1(&JsValue::NULL, &val);
        }
    }
}

/// DAP: `InitializedEvent` must be sent only after the `initialize` response was delivered,
/// and once the debug adapter is ready. Here readiness means we have a [`Debugger`] from the
/// worker's `debug` message.
fn try_emit_initialized(state: &Rc<RefCell<DapState>>) {
    let should = {
        let s = state.borrow();
        s.client_initialized && s.debugger.is_some() && !s.initialized_emitted
    };
    if should {
        state.borrow_mut().initialized_emitted = true;
        emit_event(state, "initialized", None);
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
                client_initialized: false,
                initialized_emitted: false,
                callback: None,
                vars: VariablesMap::default(),
                _closure: None,
            })),
        }
    }

    /// Attaches a worker to the adapter.
    ///
    /// Listens for the worker's `debug` message (containing `DebugInfo`) to construct
    /// the internal `Debugger`, `breakpoint` messages to emit DAP `stopped` events,
    /// and `stop` messages to emit `terminated`.
    ///
    /// **Session flags:** we reset `initialized_emitted` so each worker/run can emit `initialized`
    /// again, but we **do not** clear `client_initialized`. Reasons:
    /// - The host may send `initialize` before `run()` attaches the worker; clearing here would
    ///   prevent `initialized` from ever firing on the subsequent `debug` message.
    /// - On **re-run** (same adapter instance, new worker), the client often does **not** send
    ///   `initialize` again. Keeping `client_initialized` true means the next `debug` message can
    ///   emit `initialized` immediately without waiting for another `initialize` request. That is
    /// intentional for this embedding. A client that wants a strict fresh DAP session per run
    /// should send `initialize` again (and/or construct a new runtime so the adapter is new).
    pub fn attach(&self, worker: web_sys::Worker) {
        {
            let mut s = self.state.borrow_mut();
            s.debugger = None;
            s.initialized_emitted = false;
        }

        let state = self.state.clone();

        let closure = Closure::wrap(Box::new(move |event: web_sys::MessageEvent| {
            let data = event.data();
            let msg_type = js_sys::Reflect::get(&data, &"type".into())
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default();

            match msg_type.as_str() {
                "debug" => {
                    let info = js_sys::Reflect::get(&data, &"info".into())
                        .expect("debug message has info field");
                    let info: DebugInfo =
                        serde_wasm_bindgen::from_value(info).expect("DebugInfo deserialization");
                    state.borrow_mut().debugger = Some(Debugger::new(info));
                    try_emit_initialized(&state);
                }
                "paused" => {
                    let reason = js_sys::Reflect::get(&data, &"reason".into())
                        .expect("debug message has reason field");
                    let reason: PauseReason = serde_wasm_bindgen::from_value(reason)
                        .expect("PauseReason deserialization");
                    emit_event(
                        &state,
                        "stopped",
                        Some(json!({
                            "reason": match reason {
                                PauseReason::Breakpoint => "breakpoint",
                                PauseReason::Step => "step"
                            },
                            "threadId": 1,
                            "allThreadsStopped": true,
                        })),
                    );
                }
                "stop" => {
                    emit_event(&state, "terminated", Some(json!({ "restart": false })));
                }
                _ => {}
            }
        }) as Box<dyn FnMut(web_sys::MessageEvent)>);

        worker
            .add_event_listener_with_callback("message", closure.as_ref().unchecked_ref())
            .expect("Added message listener to worker");

        self.state.borrow_mut()._closure = Some(closure);
    }

    /// Sends a DAP request and returns the response synchronously.
    /// Events are emitted separately through the registered callback.
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

        let args = arguments.unwrap_or(Value::Null);

        let (rseq, result) = {
            let mut state = self.state.borrow_mut();
            let rseq = state.next_seq();
            let result = match command.as_str() {
                "initialize" => {
                    state.client_initialized = true;
                    state.handle_initialize()
                }
                "configurationDone" => state.handle_configuration_done(),
                "setExceptionBreakpoints" => state.handle_set_exception_breakpoints(),
                "setFunctionBreakpoints" => Ok(json!({ "breakpoints": [] })),
                "threads" => state.handle_threads(),
                "setBreakpoints" => state.handle_set_breakpoints(&args),
                "stackTrace" => state.handle_stack_trace(),
                "scopes" => state.handle_scopes(&args),
                "variables" => state.handle_variables(&args),
                "continue" => state.handle_continue(),
                "next" => state.handle_next(),
                "stepIn" => state.handle_step_in(),
                "stepOut" => state.handle_step_out(),
                "disconnect" => Ok(Value::Null),
                other => Err(anyhow::anyhow!("Unknown command: {other}")),
            };
            (rseq, result)
        }; // borrow_mut dropped here

        let response = respond(rseq, seq, &command, result);
        let ser = serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true);
        let out = response.serialize(&ser).unwrap_or(JsValue::NULL);

        if command == "initialize" {
            try_emit_initialized(&self.state);
        }

        out
    }

    /// Registers a callback that receives all DAP events.
    pub fn on(&self, callback: js_sys::Function) {
        self.state.borrow_mut().callback = Some(callback);
    }
}
