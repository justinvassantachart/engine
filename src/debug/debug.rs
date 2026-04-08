use crate::dap::types::{StackFrame, Variable};
use crate::types::{DebugInfo, WorkerOut};
use js_sys::{Object, Reflect, WebAssembly};
use wasm_bindgen::prelude::*;
use wasmer::{
    AsStoreMut, Function, FunctionEnv, FunctionEnvMut, Global, Imports, Memory, js::AsJs,
};

pub const BREAKPOINT_PREFIX_BYTES: usize = 16;

// ---------------------------------------------------------------------------
// Main-thread Debugger
// ---------------------------------------------------------------------------

/// Main-thread debugger that operates on shared memory from an attached worker.
/// Constructed from `DebugInfo` received via the worker's `debug` message.
pub struct Debugger {
    info: DebugInfo,
    sentinel: js_sys::Int32Array,
    flags: js_sys::Uint8Array,
}

impl Debugger {
    pub fn new(info: DebugInfo) -> Self {
        let sentinel = js_sys::Int32Array::new_with_byte_offset_and_length(&info.breakpoints, 0, 4);
        let flags = js_sys::Uint8Array::new_with_byte_offset(
            &info.breakpoints,
            BREAKPOINT_PREFIX_BYTES as u32,
        );
        Self {
            info,
            sentinel,
            flags,
        }
    }

    /// Walks the debug stack and returns the current call stack.
    pub fn backtrace(&self) -> Vec<StackFrame> {
        let sp = self.sentinel.get_index(3) as u32;
        let buffer = self.info.stack.memory.buffer();
        let buffer = buffer.unchecked_ref::<js_sys::ArrayBuffer>();
        let stack_top = buffer.byte_length();
        let stack_view = js_sys::DataView::new(buffer, 0, stack_top as usize);

        let mut frames = Vec::new();
        let mut pos = sp;

        while pos < stack_top {
            let func_idx = stack_view.get_uint32_endian(pos as usize, true) as usize;
            if func_idx >= self.info.functions.len() {
                break;
            }
            let func = &self.info.functions[func_idx];
            frames.push(StackFrame {
                id: frames.len() as u32,
                name: func.name.clone(),
                line: 0, // TODO: resolve from DWARF
                column: 0,
            });
            pos += func.size as u32;
        }

        frames
    }

    /// Replaces breakpoints for the given source file.
    /// Returns a list of `(line, verified)` pairs.
    pub fn set_breakpoints(&self, file: &str, lines: &[i64]) -> Vec<(i64, bool)> {
        Vec::new() // TODO
    }

    pub fn get_variables(&self) -> Vec<Variable> {
        Vec::new() // TODO: resolve variables from debug stack + DWARF
    }

    /// Resumes the worker by signaling through the sentinel.
    pub fn continue_(&self) {
        js_sys::Atomics::add(&self.sentinel, 0, 1).unwrap();
        js_sys::Atomics::notify(&self.sentinel, 0).unwrap();
    }
}

// ---------------------------------------------------------------------------
// Worker-side Debugger
// ---------------------------------------------------------------------------

/// SAFETY: In wasm32 there is no shared-memory threading; all execution is single-threaded.
unsafe impl Send for WorkerDebugger {}

/// Worker-side debugger that instruments WASM execution and blocks on breakpoints.
///
/// The breakpoint buffer is a SharedArrayBuffer laid out as:
///
/// **Bytes 0..15 — Prefix (4 x u32)**
/// - `[0]` Sentinel for pause/resume (`Atomics.wait` / `Atomics.notify`)
/// - `[1]` Mode (0 = breakpoints, 1 = step into, 2 = step over, 3 = step out)
/// - `[2]` Current breakpoint location index
/// - `[3]` Saved debug stack pointer
///
/// **Bytes 16.. — Breakpoint Enable/Disable Flags**
/// - `flags[N]` corresponds to `locations[N]` (0-based)
/// - Value 0 = disabled, >0 = number of breakpoints enabled on that location
pub struct WorkerDebugger {
    info: DebugInfo,
    stack_pointer: js_sys::WebAssembly::Global,
}

fn create_stack_pointer(info: &DebugInfo) -> Result<WebAssembly::Global, JsValue> {
    let global_desc = Object::new();

    Reflect::set(&global_desc, &"value".into(), &"i32".into())?;
    Reflect::set(&global_desc, &"mutable".into(), &true.into())?;

    let buffer = info.stack.memory.buffer();
    let size_bytes = Reflect::get(&buffer, &"byteLength".into())?;

    let global = WebAssembly::Global::new(&global_desc, &size_bytes)?;
    Ok(global)
}

impl WorkerDebugger {
    pub fn new(info: DebugInfo) -> Self {
        Self {
            stack_pointer: create_stack_pointer(&info).expect("Created stack pointer"),
            info,
        }
    }

    /// Attaches the debugger to a given WASM instance.
    /// Waits for the client to initialize the debugger.
    pub fn attach(self, store: &mut impl AsStoreMut, imports: &mut Imports) {
        self.send_debug_info();

        imports.define(
            "debug",
            "memory",
            Memory::from_jsvalue(
                store,
                &self.info.memory.ty,
                self.info.memory.memory.as_ref(),
            )
            .unwrap(),
        );

        imports.define(
            "debug",
            "stack",
            Memory::from_jsvalue(store, &self.info.stack.ty, self.info.stack.memory.as_ref())
                .unwrap(),
        );

        imports.define(
            "debug",
            "sp",
            Global::from_jsvalue(
                store,
                &wasmer::GlobalType::new(wasmer::Type::I32, wasmer::Mutability::Var),
                &self.stack_pointer,
            )
            .unwrap(),
        );

        let env = FunctionEnv::new(store, self);
        imports.define(
            "debug",
            "bkpt",
            Function::new_typed_with_env(
                store,
                &env,
                |env: FunctionEnvMut<WorkerDebugger>, index: i32| {
                    env.data().bkpt(index as usize);
                },
            ),
        );
    }

    fn send_debug_info(&self) {
        WorkerOut::Debug {
            info: self.info.clone(),
        }
        .send();
        self.wait_for_resume();
    }

    /// Check if a breakpoint at the given index is enabled
    pub fn bkpt_enabled(&self, index: usize) -> bool {
        if index > self.info.locations.len() {
            return false;
        }

        let flags = js_sys::Uint8Array::new_with_byte_offset(
            &self.info.breakpoints,
            BREAKPOINT_PREFIX_BYTES as u32,
        );
        flags.get_index(index as u32) != 0
    }

    /// Blocks until TypeScript signals resume via `Atomics.notify()` on the sentinel.
    pub fn wait_for_resume(&self) {
        let sentinel =
            js_sys::Int32Array::new_with_byte_offset_and_length(&self.info.breakpoints, 0, 1);
        let current = js_sys::Atomics::load(&sentinel, 0).unwrap_or(0);
        let _ = js_sys::Atomics::wait(&sentinel, 0, current);
    }

    /// Check if breakpoint is enabled, and if so, wait for resume.
    ///
    /// This is the main entry point called from instrumented WASM code.
    pub fn bkpt(&self, index: usize) -> bool {
        if !self.bkpt_enabled(index) {
            return false;
        }
        let sentinel =
            js_sys::Int32Array::new_with_byte_offset_and_length(&self.info.breakpoints, 0, 4);
        let sp = Reflect::get(&self.stack_pointer, &"value".into())
            .unwrap()
            .as_f64()
            .unwrap() as i32;
        js_sys::Atomics::store(&sentinel, 2, index as i32).unwrap();
        js_sys::Atomics::store(&sentinel, 3, sp).unwrap();

        WorkerOut::Breakpoint.send();
        self.wait_for_resume();
        true
    }
}
