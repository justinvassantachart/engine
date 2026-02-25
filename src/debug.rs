use crate::types::{DebugInfo, WorkerOut};
use js_sys::SharedArrayBuffer;

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
}

const SENTINEL_BYTES: u32 = 4;

impl Debugger {
    pub fn new(info: DebugInfo) -> Self {
        let buffer_size = SENTINEL_BYTES + info.locations.len() as u32;
        let buffer = SharedArrayBuffer::new(buffer_size);

        Self { info, buffer }
    }

    pub fn send_debug_info(&self) {
        WorkerOut::Debug {
            info: self.info.clone(),
            breakpoint_buffer: self.buffer.clone(),
        }
        .send();
        self.wait_for_resume();
    }

    /// Check if a breakpoint at the given index is enabled
    pub fn bkpt_enabled(&self, index: u32) -> bool {
        if index as usize > self.info.locations.len() {
            return false;
        }

        let flags = js_sys::Uint8Array::new_with_byte_offset_and_length(
            &self.buffer,
            SENTINEL_BYTES,
            self.info.locations.len() as u32,
        );
        flags.get_index(index) != 0
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

        WorkerOut::Breakpoint {
            location_index: index,
        }
        .send();

        self.wait_for_resume();
        true
    }
}
