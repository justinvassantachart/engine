use crate::types::{BreakpointMode, DebugInfo, PauseReason, WorkerOut};
use crate::util::{warning, weak_error};
use js_sys::{Object, Reflect, WebAssembly};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasmer::{
    AsStoreMut, Function, FunctionEnv, FunctionEnvMut, Global, Imports, Memory, js::AsJs,
};

/// SAFETY: In wasm32 there is no shared-memory threading; all execution is single-threaded.
unsafe impl Send for Debuggee {}

/// Worker-side debuggee that instruments WASM execution and blocks on breakpoints.
pub struct Debuggee {
    info: DebugInfo,
    stack_pointer: js_sys::WebAssembly::Global,
    stack: js_sys::DataView,
    state: js_sys::Int32Array,
    flags: js_sys::Uint8Array,
    last_sp: i32,
}

fn create_stack_pointer(
    info: &DebugInfo,
    state: &js_sys::Int32Array,
) -> Result<WebAssembly::Global, JsValue> {
    let global_desc = Object::new();

    Reflect::set(&global_desc, &"value".into(), &"i32".into())?;
    Reflect::set(&global_desc, &"mutable".into(), &true.into())?;

    let buffer = info.stack.memory.buffer();
    let size_bytes = Reflect::get(&buffer, &"byteLength".into())?;

    let global = WebAssembly::Global::new(&global_desc, &size_bytes)?;
    state.set_index(0, size_bytes.as_f64().unwrap() as i32);
    state.set_index(1, BreakpointMode::Normal.into());
    Ok(global)
}

impl Debuggee {
    pub fn new(info: DebugInfo) -> Self {
        let state = info.get_bp_state();
        let flags = info.get_bp_flags();

        let stack = info.stack.memory.buffer();
        let stack = stack.unchecked_ref::<js_sys::ArrayBuffer>();
        let stack = js_sys::DataView::new(stack, 0, stack.byte_length() as usize);

        Self {
            stack_pointer: create_stack_pointer(&info, &state).expect("Created stack pointer"),
            stack,
            state,
            flags,
            last_sp: 0,
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

        if self.info.is_multi_memory() {
            imports.define(
                "debug",
                "stack",
                Memory::from_jsvalue(store, &self.info.stack.ty, self.info.stack.memory.as_ref())
                    .unwrap(),
            );
        }

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
                |mut env: FunctionEnvMut<Debuggee>, index: i32| {
                    env.data_mut().bkpt(index as usize);
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
    fn bkpt_enabled(&self, index: usize) -> bool {
        self.flags.get_index(index as u32) != 0
    }

    fn bkpt_mode(&self) -> BreakpointMode {
        let mode = self.state.get_index(1);
        BreakpointMode::try_from(mode)
            .ok()
            .unwrap_or(BreakpointMode::Normal)
    }

    /// Blocks until the stack-pointer field changes from its current value (e.g. cleared by `continue_`).
    pub fn wait_for_resume(&self) {
        let current = js_sys::Atomics::load(&self.state, 0).unwrap_or(0);
        if current == 0 {
            return;
        }
        weak_error!(js_sys::Atomics::wait(&self.state, 0, current));
    }

    /// Decide whether execution should pause at this instrumented breakpoint.
    ///
    /// This is the main entry point called from instrumented WASM code.
    pub fn bkpt(&mut self, index: usize) -> bool {
        let sp = self.stack_pointer.value().as_f64().unwrap() as i32;

        let reason = if self.bkpt_enabled(index) {
            PauseReason::Breakpoint
        } else {
            let mode = self.bkpt_mode();
            let should_pause = match mode {
                BreakpointMode::Normal => false,
                BreakpointMode::StepInto => true,
                BreakpointMode::StepOver => sp >= self.last_sp,
                BreakpointMode::StepOut => sp > self.last_sp,
            };
            if !should_pause {
                return false;
            }
            PauseReason::Step
        };

        let pc = self
            .info
            .dwarf
            .location_at(index)
            .map(|location| location.address());

        let Some(pc) = pc else {
            warning!(
                "Could not find corresponding location for breakpoint index {:?}",
                index
            );
            return false;
        };

        // On a breakpoint hit we must write the current PC into the most recent frame.
        // This avoids having to add instrumentation code to do this on every line,
        // and instead only do it when a breakpoint is actually hit
        self.stack.set_uint32_endian(sp as usize, pc.0 as u32, true);

        self.last_sp = sp;
        js_sys::Atomics::store(&self.state, 0, sp).unwrap();

        WorkerOut::Paused { reason }.send();
        self.wait_for_resume();
        true
    }
}
