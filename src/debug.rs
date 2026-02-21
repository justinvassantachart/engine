use crate::types::{LocationInfo, WorkerOut};
use js_sys::SharedArrayBuffer;
use std::cell::RefCell;

// Thread-local storage for the global debugger instance
thread_local! {
    static DEBUGGER: RefCell<Option<Debugger>> = RefCell::new(None);
}

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
/// The instrumented WASM uses 1-based indices: `bkpt(N)` checks `flags[N-1]`.
pub struct Debugger {
    locations: Vec<LocationInfo>,
    files: Vec<String>,
    buffer: SharedArrayBuffer,
}

const SENTINEL_BYTES: u32 = 4;

impl Debugger {
    pub fn new(locations: Vec<LocationInfo>, files: Vec<String>) -> Self {
        let buffer_size = SENTINEL_BYTES + locations.len() as u32;
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
            locations: self.locations.clone(),
            files: self.files.clone(),
            breakpoint_buffer: self.buffer.clone(),
        }
        .send();
    }

    /// Check if a breakpoint at the given index is enabled.
    /// Index is 1-based (from instrumented WASM). Returns false for 0 or out-of-bounds.
    pub fn bkpt_enabled(&self, index: u32) -> bool {
        if index == 0 || index as usize > self.locations.len() {
            return false;
        }

        let flags = js_sys::Uint8Array::new_with_byte_offset_and_length(
            &self.buffer,
            SENTINEL_BYTES,
            self.locations.len() as u32,
        );
        flags.get_index(index - 1) != 0
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
    pub fn bkpt(&self, index: u32) -> bool {
        if !self.bkpt_enabled(index) {
            return false;
        }

        WorkerOut::BreakpointHit {
            location_index: index - 1,
        }
        .send();

        self.wait_for_resume();
        true
    }

    /// Set the global debugger instance.
    /// Call this before running instrumented code.
    pub fn set_global(debugger: Debugger) {
        DEBUGGER.with(|d| *d.borrow_mut() = Some(debugger));
    }

    /// Handle a breakpoint hit from WASM import.
    /// This is the function provided as the "debug"."bkpt" import.
    pub fn handle_bkpt(index: i32) {
        DEBUGGER.with(|d| {
            if let Some(debugger) = d.borrow().as_ref() {
                debugger.bkpt(index as u32);
            }
        });
    }
}
