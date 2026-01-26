use js_sys::SharedArrayBuffer;
use crate::types::{LocationInfo, WorkerOut};

/// Debugger state that manages breakpoint locations and their enable/disable state.
/// 
/// The breakpoint buffer is a SharedArrayBuffer with two purposes:
/// 
/// **Index 0 — Pause/Resume Signal (Sentinel)**
/// - When a breakpoint is hit, the Rust side calls `Atomics.wait()` on index 0
/// - This blocks execution until TypeScript calls `Atomics.notify()` on index 0
/// - Value doesn't matter; we just wait for the notification
/// 
/// **Index 1..N — Breakpoint Enable/Disable Flags**
/// - Index N corresponds to `locations[N-1]`
/// - Value 0 = breakpoint disabled
/// - Value 1 = breakpoint enabled
pub struct Debugger {
    locations: Vec<LocationInfo>,
    files: Vec<String>,
    buffer: SharedArrayBuffer,
}

impl Debugger {
    pub fn new(locations: Vec<LocationInfo>, files: Vec<String>) -> Self {
        let buffer_size = (locations.len() + 1) as u32;
        let buffer = SharedArrayBuffer::new(buffer_size);
        
        Self {
            locations,
            files,
            buffer,
        }
    }

    pub fn buffer(&self) -> &SharedArrayBuffer {
        &self.buffer
    }

    pub fn locations(&self) -> &[LocationInfo] {
        &self.locations
    }

    pub fn files(&self) -> &[String] {
        &self.files
    }

    pub fn send_debug_info(&self) {
        WorkerOut::Debug {
            breakpoints: self.locations.clone(),
            files: self.files.clone(),
            breakpoint_buffer: self.buffer.clone(),
        }.send();
    }

    /// Check if a breakpoint at the given index is enabled.
    /// 
    /// This reads from the SharedArrayBuffer using atomic operations.
    /// Returns false for index 0 (sentinel) or out-of-bounds indices.
    pub fn bkpt_enabled(&self, index: u32) -> bool {
        if index == 0 || index as usize > self.locations.len() {
            return false;
        }
        
        let view = js_sys::Int8Array::new(&self.buffer);
        let value = js_sys::Atomics::load(&view, index).unwrap_or(0);
        value != 0
    }

    /// Called when a breakpoint is hit. Blocks until TypeScript signals resume.
    /// 
    /// This waits on index 0 (the sentinel) using `Atomics.wait()`.
    /// TypeScript will call `Atomics.notify()` on index 0 when the user
    /// wants to resume execution.
    /// 
    /// The `expected_value` parameter is the current value at index 0.
    /// If TypeScript has already changed it, the wait returns immediately.
    pub fn wait_for_resume(&self) {
        let view = js_sys::Int32Array::new(&self.buffer);
        let current = js_sys::Atomics::load(&view, 0).unwrap_or(0);
        // Wait until TypeScript notifies us
        let _ = js_sys::Atomics::wait(&view, 0, current);
    }

    /// Check if breakpoint is enabled, and if so, wait for resume.
    /// 
    /// This is the main entry point called from instrumented WASM code.
    pub fn bkpt(&self, index: u32) -> bool {
        if !self.bkpt_enabled(index) {
            return false;
        }
        
        // TODO: Send BreakpointHit message to TypeScript with stack info
        
        self.wait_for_resume();
        true
    }
}
