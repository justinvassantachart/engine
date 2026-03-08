use serde::{Deserialize, Serialize};
use serde_repr::Serialize_repr;
use std::collections::{HashMap, HashSet};
use tsify::Tsify;
use wasm_bindgen::JsValue;
use web_sys::DedicatedWorkerGlobalScope;

#[derive(Debug, Tsify, Deserialize)]
#[serde(untagged)]
pub enum FsNode {
    File(String),
    Dir(HashMap<String, FsNode>),
}

#[derive(Debug, Tsify, Deserialize)]
pub struct WorkerStart {
    pub fs: HashMap<String, FsNode>,

    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub stdin_buffer: js_sys::SharedArrayBuffer,
    pub is_debug: bool,
}

#[derive(Clone, Copy, Debug, Tsify, Serialize_repr)]
#[repr(u8)]
#[tsify(type = "1 | 2")]
pub enum StdoutMode {
    Out = 1,
    Err = 2,
}

#[derive(Tsify, Serialize)]
#[serde(tag = "type")]
pub enum WorkerOut<'a> {
    #[serde(rename = "ready")]
    Ready,
    #[serde(rename = "stdout")]
    Stdout {
        #[tsify(type = "Uint8Array")]
        #[serde(with = "serde_bytes")]
        data: &'a [u8],
        mode: StdoutMode,
    },
    #[serde(rename = "debug")]
    Debug {
        info: DebugInfo,
        /// Bitfield where index N corresponds to breakpoint N.
        /// Index 0 is a sentinel (always 0). Length = breakpoints.len() + 1.
        #[serde(with = "serde_wasm_bindgen::preserve")]
        breakpoint_buffer: js_sys::SharedArrayBuffer,

        /// The main memory of the executing program
        #[serde(with = "serde_wasm_bindgen::preserve")]
        memory: js_sys::WebAssembly::Memory,
    },

    /// Request the main thread to trigger a file download (workers have no document/window).
    #[serde(rename = "download")]
    Download {
        #[tsify(type = "Uint8Array")]
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
        filename: String,
    },

    /// Sent when execution pauses at an enabled breakpoint.
    #[serde(rename = "breakpoint")]
    Breakpoint {
        /// 0-based index into [DebugInfo::locations] array
        location_index: usize,
    },

    #[serde(rename = "stop")]
    Stop,
}

impl<'a> WorkerOut<'a> {
    pub fn send(&self) {
        let scope = DedicatedWorkerGlobalScope::from(JsValue::from(js_sys::global()));
        scope
            .post_message(
                &serde_wasm_bindgen::to_value(self)
                    .expect("serialization worked")
                    .into(),
            )
            .expect("post_message succeeded");
    }
}

#[derive(Debug, Clone, Tsify, Serialize)]
pub struct LocationInfo {
    /// Index into [DebugInfo::files]
    pub file: usize,
    pub line: usize,
    pub col: usize,
    /// Byte offset into the WASM code section for instrumentation
    pub address: usize,
}

/// Debug information parsed from DWARF
#[derive(Debug, Clone, Default, Tsify, Serialize)]
pub struct DebugInfo {
    pub memory: MemoryInfo,
    /// Breakpoint locations (file index, line, col, WASM address).
    pub locations: Vec<LocationInfo>,
    /// Deduplicated source filenames; index matches `LocationInfo::file`.
    pub files: Vec<String>,
    pub functions: Vec<DebugFunction>,
    pub types: Vec<DebugType>,
}

#[derive(Debug, Clone, Tsify, Serialize)]
pub struct MemoryInfo {
    pub main: wasmer::MemoryType,
    pub debug: wasmer::MemoryType,
}

impl Default for MemoryInfo {
    fn default() -> Self {
        Self {
            main: wasmer::MemoryType::new(1, Some(1), true),
            // Default debug stack size is 64 pages ~ 4MiB
            debug: wasmer::MemoryType::new(64, Some(64), true),
        }
    }
}

/// A single DWARF expression operation, converted from `gimli::Operation`
#[derive(Debug, Clone)]
pub enum DwarfOp {
    /// Push a value stored in a WASM location.
    Wasm(WasmOp),

    /// `DW_OP_fbreg +offset`: push frame_base + offset.
    /// The instrumenter inlines the function's `frame_base` ops, then adds the offset.
    FrameOffset { offset: i64 },

    /// `DW_OP_stack_value`: marks the expression result as a value, not an address.
    /// Without this, the instrumenter dereferences the result via `i32.load`.
    StackValue,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum WasmOp {
    /// The index of a local in the currently executing function.
    Local(usize),
    /// The index of a global.
    Global(usize),
    /// The index of an item on the operand stack. 0 is the bottom of the operand stack.
    Operand(usize),
}

#[derive(Default, Debug, Clone)]
pub struct VarLocation(pub Vec<VarLocationRange>);

impl VarLocation {
    pub fn is_optimized_out(&self) -> bool {
        self.0.is_empty()
    }

    pub fn location_at(&self, addr: usize) -> Option<&VarLocationRange> {
        self.0
            .iter()
            .find(|range| addr >= range.start && addr < range.end)
    }
}

/// A location expression valid over a specific PC range.
/// At `-O0` a variable typically has one range spanning the whole function.
/// At higher optimization levels, DWARF location lists produce multiple ranges
/// as the variable moves between locals, memory, or gets optimized out.
#[derive(Debug, Clone)]
pub struct VarLocationRange {
    /// Code-section-relative start PC (inclusive).
    pub start: usize,
    /// Code-section-relative end PC (exclusive).
    pub end: usize,
    /// DWARF operations that produce the variable's value.
    pub ops: Vec<DwarfOp>,
}

#[derive(Debug, Clone, Tsify, Serialize)]
pub struct DebugFunction {
    pub name: String,
    /// Code section offset of the start of the function
    pub address: usize,
    pub variables: Vec<DebugVariable>,
    #[serde(skip)]
    pub frame: DebugFrame,
}

#[derive(Debug, Clone)]
pub struct DebugFrame {
    /// The total size in bytes of the stack frame, including it's 32-bit tag
    pub size: usize,
    /// DWARF expression for the function's frame base (`DW_AT_frame_base`)
    pub base: VarLocation,
    /// The entries in this stack frame
    pub layout: Vec<DebugFrameEntry>,
}

impl DebugFrame {
    /// Clears the layout of the stack frame and resets it to its minimum size.
    pub fn reset(&mut self) {
        self.size = 4; // Space for debug function index tag
        self.layout.clear();
    }

    /// Ensures an entry exists for `loc` and `ty`, adds `bkpt` to its lifetime, and returns its offset.
    /// Returns [None] if an entry could not be created (e.g. for ref types).
    pub fn place(&mut self, loc: WasmOp, ty: wasmparser::ValType, bkpt: usize) -> Option<usize> {
        use wasmparser::ValType;
        if matches!(ty, ValType::Ref(_)) {
            return None;
        }
        if let Some(pos) = self
            .layout
            .iter()
            .position(|e| e.location == loc && e.ty == ty)
        {
            self.layout[pos].lifetime.insert(bkpt);
            return Some(self.layout[pos].offset);
        }

        let offset = self.size;
        let size = match ty {
            ValType::I32 | ValType::F32 => 4,
            ValType::I64 | ValType::F64 => 8,
            ValType::V128 => 16,
            ValType::Ref(_) => unreachable!(),
        };
        self.size += size;
        let mut entry = DebugFrameEntry {
            offset,
            ty,
            location: loc,
            lifetime: HashSet::default(),
        };
        entry.lifetime.insert(bkpt);
        self.layout.push(entry);
        Some(offset)
    }
}

#[derive(Debug, Clone)]
pub struct DebugFrameEntry {
    /// The byte offset of this entry in its containing stack frame
    pub offset: usize,
    /// The WebAssembly type of the value stored by the entry
    pub ty: wasmparser::ValType,
    /// The WebAssembly location (local, global, or stack) represented by this entry's value
    pub location: WasmOp,
    /// A list of breakpoint indices for which this entry is considered valid.
    ///
    /// If a breakpoint index `N` is contained in this list, then accessing this entry's
    /// value immediately after hitting breakpoint `N` will yield a valid value.
    ///
    /// More than one [DebugFrameEntry] in a frame may share the same [DebugFrameEntry::location],
    /// but they are guaranteed to never have overlapping values in [DebugFrameEntry::lifetime].
    /// This permits two frame entries with the same location to contain different types
    /// at different points during the function's execution – for example, `WasmOp::Stack(0)`
    /// might have type [wasmparser::ValType::I32] at the beginning of a function, but change to
    /// [wasmparser::ValType::F64] later on in the function as values are shifted on and off
    /// the operand stack.
    pub lifetime: HashSet<usize>,
}

#[derive(Debug, Clone, Tsify, Serialize)]
pub struct DebugVariable {
    /// Index into [DebugInfo::types]
    pub ty: usize,
    pub name: String,
    /// Where and when the variable's value can be read.
    /// Empty if the variable is always optimized out.
    #[serde(skip)]
    pub location: VarLocation,
}

#[derive(Debug, Clone, Copy, Tsify, Serialize)]
pub enum TypeEncoding {
    Signed,
    Unsigned,
    Float,
    Bool,
    Address,
    Unknown,
}

#[derive(Debug, Clone, Tsify, Serialize)]
pub struct DebugType {
    pub name: String,
    pub byte_size: usize,
    // useful for showing the type in the debugger
    pub encoding: TypeEncoding,
    pub offset: usize,
    pub fields: Vec<DebugType>,
}
