//! Compiler-only import adapter. It is deliberately not installed for guest code.
//! Installing this adapter does not itself authorize object admission; only the
//! verified build-fixed toolchain profile and validated invocation may do that.
use std::sync::{Arc, Mutex};
use wasmer::{Function, FunctionEnv, FunctionEnvMut, Imports, Instance, Memory, Store};

use super::cpp_receipt::{Bypass, Capture};

pub type InvocationCapture = Arc<Mutex<Capture>>;

pub struct ReceiptImport {
    memory: Option<Memory>,
    capture: Option<InvocationCapture>,
}

impl ReceiptImport {
    /// `None` supplies the required declining adapter for other compiler tools,
    /// including wasm-ld. Every call returns without reading guest memory.
    pub fn attach(store: &mut Store, imports: &mut Imports, capture: Option<InvocationCapture>) -> FunctionEnv<Self> {
        let environment = FunctionEnv::new(store, Self { memory: None, capture });
        imports.define("webide_compiler", "cache_receipt_v1", Function::new_typed_with_env(
            store, &environment, |environment: FunctionEnvMut<Self>, pointer: i32, length: i32| -> i32 {
                let Some(capture) = &environment.data().capture else { return 1; };
                let Ok(mut capture) = capture.lock() else { return 1; };
                if capture.attempted() { return capture.decline(Bypass::DuplicateReceipt); }
                let Some(memory) = &environment.data().memory else { return capture.decline(Bypass::MalformedReceipt); };
                let Ok(pointer) = u32::try_from(pointer) else { return capture.decline(Bypass::MalformedReceipt); };
                let Ok(length) = u32::try_from(length) else { return capture.decline(Bypass::MalformedReceipt); };
                if length as usize > capture.limit() { return capture.decline(Bypass::ReceiptBudget); }
                let Some(end) = pointer.checked_add(length) else { return capture.decline(Bypass::MalformedReceipt); };
                let view = memory.view(&environment);
                if u64::from(end) > view.data_size() { return capture.decline(Bypass::MalformedReceipt); }
                let mut bytes = Vec::new();
                if bytes.try_reserve_exact(length as usize).is_err() { return capture.decline(Bypass::ReceiptBudget); }
                bytes.resize(length as usize, 0);
                if view.read(u64::from(pointer), &mut bytes).is_err() { return capture.decline(Bypass::MalformedReceipt); }
                // Only retain the bounded owned bytes here. Parsing waits until
                // normal compiler exit and diagnostic/object checks. No borrowed
                // memory, asynchronous observer or filesystem output escapes.
                capture.receive_owned(bytes)
            },
        ));
        environment
    }

    /// Call after instantiation and before invoking _start. A compiler invoking
    /// the callback from its Wasm start section is deliberately declined.
    pub fn bind(environment: &FunctionEnv<Self>, store: &mut Store, instance: &Instance) {
        environment.as_mut(store).memory = instance.exports.get_memory("memory").ok().cloned();
    }
}
