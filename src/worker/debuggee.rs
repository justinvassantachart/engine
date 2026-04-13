use crate::types::{DebugInfo, WorkerOut};
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
    state: js_sys::Int32Array,
    flags: js_sys::Uint8Array,
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
    state.set_index(1, 0);
    Ok(global)
}

impl Debuggee {
    pub fn new(info: DebugInfo) -> Self {
        let state = info.get_bp_state();
        let flags = info.get_bp_flags();
        Self {
            stack_pointer: create_stack_pointer(&info, &state).expect("Created stack pointer"),
            state,
            flags,
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
                |env: FunctionEnvMut<Debuggee>, index: i32| {
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
        self.flags.get_index(index as u32) != 0
    }

    /// Blocks until the stack-pointer field changes from its current value (e.g. cleared by `continue_`).
    pub fn wait_for_resume(&self) {
        let current = js_sys::Atomics::load(&self.state, 0).unwrap_or(0);
        if current == 0 {
            return;
        }
        weak_error!(js_sys::Atomics::wait(&self.state, 0, current));
    }

    /// Check if breakpoint is enabled, and if so, wait for resume.
    ///
    /// This is the main entry point called from instrumented WASM code.
    pub fn bkpt(&self, index: usize) -> bool {
        if !self.bkpt_enabled(index) {
            return false;
        }
        let sp = self.stack_pointer.value().as_f64().unwrap() as i32;

        let pc = self
            .info
            .dwarf
            .location_at(index)
            .map(|location| location.line.address());

        let Some(pc) = pc else {
            warning!(
                "Could not find corresponding location for breakpoint index {:?}",
                index
            );
            return false;
        };

        let stack_buffer = self.info.stack.memory.buffer();
        let stack_buffer = stack_buffer.unchecked_ref::<js_sys::ArrayBuffer>();
        let stack_view =
            js_sys::DataView::new(stack_buffer, 0, stack_buffer.byte_length() as usize);
        stack_view.set_uint32_endian(sp as usize, pc.0 as u32, true);

        js_sys::Atomics::store(&self.state, 0, sp).unwrap();
        WorkerOut::Breakpoint.send();
        self.wait_for_resume();
        true
    }
}
