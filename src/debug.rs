use crate::types::{DebugInfo, WorkerOut};
use js_sys::{Object, Reflect, SharedArrayBuffer, WebAssembly};
use wasm_bindgen::prelude::*;
use wasmer::{
    AsStoreMut, Function, FunctionEnv, FunctionEnvMut, Global, Imports, Memory, js::AsJs,
};

/// SAFETY: In wasm32 there is no shared-memory threading; all execution is single-threaded.
unsafe impl Send for Debugger {}

/// Debugger state that manages breakpoint locations and their enable/disable state.
///
/// The breakpoint buffer is a SharedArrayBuffer with two regions:
///
/// **Bytes 0..3 — Pause/Resume Signal (Sentinel)**
/// Viewed as `Int32Array(buffer, 0, 1)`.
/// - When a breakpoint is hit, Rust calls `Atomics.wait()` on element 0
/// - This blocks until TypeScript calls `Atomics.notify()` to resume
///
/// **Bytes 4.. — Breakpoint Enable/Disable Flags**
/// Viewed as `Uint8Array(buffer, 4)`.
/// - flags[N] corresponds to `locations[N]` (0-based)
/// - Value 0 = disabled, >0 = number of breakpoints enabled on that location
///
/// The instrumented WASM uses 0-based indices: `bkpt(N)` checks `flags[N]`.
pub struct Debugger {
    info: DebugInfo,
    buffer: SharedArrayBuffer,
    /// The main program memory
    main_memory: js_sys::WebAssembly::Memory,
    /// The memory which holds the debug stack during execution
    debug_memory: js_sys::WebAssembly::Memory,
    /// The location of the debug stack pointer.
    /// Points to the start of the current function's stack frame.
    stack_pointer: js_sys::WebAssembly::Global,
}

const SENTINEL_BYTES: u32 = 4;

fn create_memory(memory: wasmer::MemoryType) -> Result<WebAssembly::Memory, JsValue> {
    let memory_desc = Object::new();

    Reflect::set(&memory_desc, &"initial".into(), &memory.minimum.0.into())?;

    if let Some(maximum) = memory.maximum {
        Reflect::set(&memory_desc, &"maximum".into(), &maximum.0.into())?;
    }

    Reflect::set(&memory_desc, &"shared".into(), &memory.shared.into())?;

    let memory = WebAssembly::Memory::new(&memory_desc)?;

    Ok(memory)
}

fn create_stack_pointer(info: &DebugInfo) -> Result<WebAssembly::Global, JsValue> {
    let global_desc = Object::new();

    Reflect::set(&global_desc, &"value".into(), &"i32".into())?;
    Reflect::set(&global_desc, &"mutable".into(), &true.into())?;
    let global =
        WebAssembly::Global::new(&global_desc, &info.memory.debug.minimum.bytes().0.into())?;

    Ok(global)
}

impl Debugger {
    pub fn new(info: DebugInfo) -> Self {
        let buffer_size = (SENTINEL_BYTES as usize) + info.locations.len();
        let buffer = SharedArrayBuffer::new(buffer_size as u32);

        Self {
            main_memory: create_memory(info.memory.main).expect("Created program memory"),
            debug_memory: create_memory(info.memory.debug).expect("Created debug memory"),
            stack_pointer: create_stack_pointer(&info).expect("Created stack pointer"),
            info,
            buffer,
        }
    }

    /// Attaches the debugger to a given WASM instance.
    /// Waits for the client to initialize the debugger.
    pub fn attach(self, store: &mut impl AsStoreMut, imports: &mut Imports) {
        self.send_debug_info();

        imports.define(
            "debug",
            "memory",
            Memory::from_jsvalue(store, &self.info.memory.main, &self.main_memory).unwrap(),
        );

        imports.define(
            "debug",
            "stack",
            Memory::from_jsvalue(store, &self.info.memory.debug, &self.debug_memory).unwrap(),
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
                |env: FunctionEnvMut<Debugger>, index: i32| {
                    env.data().bkpt(index as usize);
                },
            ),
        );
    }

    fn send_debug_info(&self) {
        WorkerOut::Debug {
            info: self.info.clone(),
            breakpoint_buffer: self.buffer.clone(),
            memory: self.main_memory.clone(),
        }
        .send();
        self.wait_for_resume();
    }

    /// Check if a breakpoint at the given index is enabled
    pub fn bkpt_enabled(&self, index: usize) -> bool {
        if index > self.info.locations.len() {
            return false;
        }

        let flags = js_sys::Uint8Array::new_with_byte_offset_and_length(
            &self.buffer,
            SENTINEL_BYTES,
            self.info.locations.len() as u32,
        );
        flags.get_index(index as u32) != 0
    }

    /// Blocks until TypeScript signals resume via `Atomics.notify()` on the sentinel.
    pub fn wait_for_resume(&self) {
        let sentinel = js_sys::Int32Array::new_with_byte_offset_and_length(&self.buffer, 0, 1);
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

        WorkerOut::Breakpoint {
            location_index: index,
        }
        .send();

        self.wait_for_resume();
        true
    }
}
