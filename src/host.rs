//! Debug host API exposed to the main thread.
//!
//! Runs in the main-thread (browser) WASM runtime. Receives DebugInfo from the worker
//! and provides get_frames() to unwrap the debug stack when paused.
//! The worker saves the stack pointer into the breakpoints buffer before sending a
//! Breakpoint message so the host can read it (the host has no access to the SP global).

use crate::debug::BREAKPOINT_PREFIX_BYTES;
use crate::dwarf::to_dwarf;
use crate::types::{DebugFunction, DebugInfo, WasmLocation};
use gimli::{EndianSlice, LittleEndian, Reader};
use js_sys::Reflect;
use wasm_bindgen::prelude::*;

// ============================================================================
// Public types
// ============================================================================

/// A resolved variable value for a single frame.
#[wasm_bindgen]
pub struct Variable {
    name: String,
    /// C type name (e.g. "int", "float*").
    ty: String,
    /// Formatted value (e.g. "42", "3.14", "0x00ff1234").
    value: String,
}

#[wasm_bindgen]
impl Variable {
    #[wasm_bindgen(getter)]
    pub fn name(&self) -> String {
        self.name.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn ty(&self) -> String {
        self.ty.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn value(&self) -> String {
        self.value.clone()
    }
}

/// A single frame in the call stack when execution is paused.
#[wasm_bindgen]
pub struct StackFrame {
    name: String,
    /// Index into DebugInfo.functions (order from DWARF parsing, not source order).
    function: u32,
    variables: Vec<Variable>,
}

#[wasm_bindgen]
impl StackFrame {
    #[wasm_bindgen(getter)]
    pub fn name(&self) -> String {
        self.name.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn function(&self) -> u32 {
        self.function
    }

    #[wasm_bindgen(getter)]
    pub fn variables(&self) -> js_sys::Array {
        let arr = js_sys::Array::new();
        for v in &self.variables {
            arr.push(&JsValue::from(Variable {
                name: v.name.clone(),
                ty: v.ty.clone(),
                value: v.value.clone(),
            }));
        }
        arr
    }
}

#[wasm_bindgen]
pub struct DebugHost {
    info: DebugInfo,
}

#[wasm_bindgen]
impl DebugHost {
    #[wasm_bindgen(constructor)]
    pub fn new(info: JsValue) -> Result<DebugHost, JsValue> {
        let info: DebugInfo =
            serde_wasm_bindgen::from_value(info).map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(DebugHost { info })
    }

    #[wasm_bindgen]
    pub fn get_frames(&self) -> Vec<StackFrame> {
        // Breakpoints buffer layout (first 16 bytes = 4 u32s):
        //   [0] Sentinel (pause/resume), [1] Mode, [2] Breakpoint index, [3] Saved SP
        let meta = js_sys::Uint32Array::new_with_byte_offset_and_length(
            &self.info.breakpoints,
            0,
            BREAKPOINT_PREFIX_BYTES as u32 / 4,
        );
        let sp = meta.get_index(3);
        if sp == 0 {
            return vec![]; // Not paused, or worker didn't write SP yet
        }
        let bkpt_idx = meta.get_index(2) as usize;

        // Stack memory is WebAssembly.Memory; get its underlying buffer (SharedArrayBuffer).
        let stack_buf = Reflect::get(self.info.stack.memory.as_ref(), &"buffer".into())
            .unwrap_or(JsValue::NULL);
        let stack_len: u32 = Reflect::get(&stack_buf, &"byteLength".into())
            .ok()
            .and_then(|v| v.as_f64())
            .map(|f| f as u32)
            .unwrap_or(0);
        if stack_len == 0 {
            return vec![];
        }

        // Rebuild the DWARF reader once; reused for every frame in this call.
        let dwarf = to_dwarf(&self.info.dwarf);

        // Program linear memory buffer (for reading C stack variables).
        let prog_buf = Reflect::get(self.info.memory.memory.as_ref(), &"buffer".into())
            .unwrap_or(JsValue::NULL);

        let mut frames = Vec::new();
        let mut offset: u32 = sp;

        // Each frame: [func_idx: u32][saved locals…]. Stack grows down;
        // caller frames are at higher addresses, so we advance offset += size.
        while offset + 4 <= stack_len {
            let u32_view =
                js_sys::Uint32Array::new_with_byte_offset_and_length(&stack_buf, offset, 1);
            let func_idx = u32_view.get_index(0);
            let Some(debug_fn) = self.info.functions.get(func_idx as usize) else {
                break;
            };

            let size = debug_fn.size as u32;

            // Variables are resolved lazily via get_variables_for_frame() when the frame is expanded.
            frames.push(StackFrame {
                name: debug_fn.name.clone(),
                function: func_idx,
                variables: vec![],
            });

            offset += size;
            if size == 0 {
                break;
            }
        }

        frames
    }

    /// Returns variables for the frame at the given index (0 = innermost). Called lazily when the
    /// UI expands a frame. Returns an empty array if not paused or frame_index is out of range.
    #[wasm_bindgen]
    pub fn get_variables_for_frame(&self, frame_index: usize) -> js_sys::Array {
        let meta = js_sys::Uint32Array::new_with_byte_offset_and_length(
            &self.info.breakpoints,
            0,
            BREAKPOINT_PREFIX_BYTES as u32 / 4,
        );
        let sp = meta.get_index(3);
        if sp == 0 {
            return js_sys::Array::new();
        }
        let bkpt_idx = meta.get_index(2) as usize;

        let stack_buf = Reflect::get(self.info.stack.memory.as_ref(), &"buffer".into())
            .unwrap_or(JsValue::NULL);
        let stack_len: u32 = Reflect::get(&stack_buf, &"byteLength".into())
            .ok()
            .and_then(|v| v.as_f64())
            .map(|f| f as u32)
            .unwrap_or(0);
        if stack_len == 0 {
            return js_sys::Array::new();
        }

        let prog_buf = Reflect::get(self.info.memory.memory.as_ref(), &"buffer".into())
            .unwrap_or(JsValue::NULL);
        let dwarf = to_dwarf(&self.info.dwarf);

        let mut offset: u32 = sp;
        let mut current = 0usize;
        while offset + 4 <= stack_len {
            let u32_view =
                js_sys::Uint32Array::new_with_byte_offset_and_length(&stack_buf, offset, 1);
            let func_idx = u32_view.get_index(0);
            let Some(debug_fn) = self.info.functions.get(func_idx as usize) else {
                break;
            };
            let size = debug_fn.size as u32;

            if current == frame_index {
                let variables =
                    get_frame_variables(&dwarf, debug_fn, &stack_buf, &prog_buf, offset, bkpt_idx, frame_index);
                let arr = js_sys::Array::new();
                for v in variables {
                    arr.push(&JsValue::from(Variable {
                        name: v.name,
                        ty: v.ty,
                        value: v.value,
                    }));
                }
                return arr;
            }

            current += 1;
            offset += size;
            if size == 0 {
                break;
            }
        }
        js_sys::Array::new()
    }
}

// ============================================================================
// DWARF-based variable reconstruction
// ============================================================================

/// Internal representation of how a variable's value should be interpreted.
#[derive(PartialEq)]
enum TypeKind {
    Signed,
    Unsigned,
    Float,
    Bool,
    Pointer,
    Other,
}

/// Where a DWARF variable's value lives at runtime.
enum VarLoc {
    /// Directly in a WASM local, global, or operand slot.
    Wasm(WasmLocation),
    /// In linear memory at `frame_base_global_value + offset`.
    /// The frame base global index comes from DW_AT_frame_base on the subprogram.
    FrameRelative(i64),
    /// In linear memory at `global_value + offset`.
    /// The global index is taken directly from the location expression.
    GlobalPlusOffset { global_idx: usize, offset: u64 },
}

/// Walk all DWARF units to find the subprogram whose `DW_AT_low_pc` matches
/// `debug_fn.address`, then collect and return its variables.
fn get_frame_variables(
    dwarf: &gimli::Dwarf<EndianSlice<'_, LittleEndian>>,
    debug_fn: &DebugFunction,
    stack_buf: &JsValue,
    prog_buf: &JsValue,
    frame_sp: u32,
    bkpt_idx: usize,
    frame_index: usize,
) -> Vec<Variable> {
    crate::log!(
        "[host] get_frame_variables: fn addr=0x{:x}, bkpt={}, layout_entries={}, frame_index={}",
        debug_fn.address,
        bkpt_idx,
        debug_fn.layout.len(),
        frame_index
    );
    let mut units = dwarf.units();
    while let Ok(Some(header)) = units.next() {
        let Ok(unit) = dwarf.unit(header) else { continue };
        let Ok(mut tree) = unit.entries_tree(None) else { continue };
        let Ok(root) = tree.root() else { continue };
        if let Some(vars) =
            search_subprogram(dwarf, &unit, root, debug_fn, stack_buf, prog_buf, frame_sp, bkpt_idx, frame_index)
        {
            crate::log!("[host] found {} variable(s) for fn 0x{:x}", vars.len(), debug_fn.address);
            return vars;
        }
    }
    crate::log!("[host] no matching subprogram found for fn addr=0x{:x}", debug_fn.address);
    vec![]
}

/// Recursively search the entries tree for the matching subprogram.
fn search_subprogram<R: Reader>(
    dwarf: &gimli::Dwarf<R>,
    unit: &gimli::Unit<R>,
    node: gimli::EntriesTreeNode<R>,
    debug_fn: &DebugFunction,
    stack_buf: &JsValue,
    prog_buf: &JsValue,
    frame_sp: u32,
    bkpt_idx: usize,
    frame_index: usize,
) -> Option<Vec<Variable>> {
    let tag = node.entry().tag();
    let is_subprogram = tag == gimli::DW_TAG_subprogram;

    // Read low_pc while the entry is still available (before node.children() moves it).
    let low_pc: Option<usize> = if is_subprogram {
        node.entry()
            .attr(gimli::DW_AT_low_pc)
            .and_then(|a| match a.value() {
                gimli::AttributeValue::Addr(addr) => Some(addr as usize),
                _ => None,
            })
    } else {
        None
    };

    if is_subprogram && low_pc == Some(debug_fn.address) {
        // Read the frame-base global index from DW_AT_frame_base before consuming children.
        let mut frame_base_global = frame_base_global_from_entry(node.entry(), unit.encoding());
        if frame_base_global.is_none() {
            // Fallback: Emscripten and similar toolchains use global 0 for __stack_pointer.
            // When DW_AT_frame_base is missing or uses an unsupported format (e.g. DW_OP_call_frame_cfa,
            // DW_OP_WASM_location with local/stack), use global 0 so FrameRelative variables still work.
            frame_base_global = Some(0);
            crate::log!("[host] subprogram frame_base_global: None, using fallback 0 (__stack_pointer)");
        } else {
            crate::log!("[host] subprogram frame_base_global: {:?}", frame_base_global);
        }

        // Found our function — collect variable children.
        let is_caller_frame = frame_index > 0;
        let mut vars = Vec::new();
        let mut children = node.children();
        while let Ok(Some(child)) = children.next() {
            let child_tag = child.entry().tag();
            crate::log!("[host] subprogram child tag: {}", child_tag);
            if matches!(
                child_tag,
                gimli::DW_TAG_variable | gimli::DW_TAG_formal_parameter
            ) {
                if let Some(v) = make_variable(
                    dwarf, unit, child.entry(), debug_fn,
                    stack_buf, prog_buf, frame_sp, bkpt_idx, frame_base_global, is_caller_frame,
                ) {
                    vars.push(v);
                }
            }
        }
        return Some(vars);
    }

    // For container nodes (namespace, module, …), recurse.
    // For other subprograms, don't recurse (they're separate frames).
    if !is_subprogram {
        let mut children = node.children();
        while let Ok(Some(child)) = children.next() {
            if let Some(result) =
                search_subprogram(dwarf, unit, child, debug_fn, stack_buf, prog_buf, frame_sp, bkpt_idx, frame_index)
            {
                return Some(result);
            }
        }
    }

    None
}

/// Extract the global index used as the frame base from a subprogram's DW_AT_frame_base.
/// Typical WASM DWARF: DW_OP_WASM_location 0x03 sp_global_idx.
fn frame_base_global_from_entry<R: Reader>(
    entry: &gimli::DebuggingInformationEntry<R>,
    encoding: gimli::Encoding,
) -> Option<usize> {
    let a = entry.attr(gimli::DW_AT_frame_base)?;
    match a.value() {
        gimli::AttributeValue::Exprloc(expr) => {
            let mut ops = expr.operations(encoding);
            match ops.next().ok()?? {
                gimli::Operation::WasmGlobal { index } => Some(index as usize),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Look up a saved global in the debug frame, use its value as a base address,
/// add `mem_offset`, and read a C-typed value from program linear memory.
///
/// When `is_caller_frame` is true (frame_index > 0), we relax the lifetime check for
/// layout lookups: the current bkpt was hit in a callee, so this frame's slots were
/// never written at this stop — but they still hold values from when we last hit a
/// breakpoint in this function (or from the prologue). We use them anyway.
fn read_from_linear_memory(
    name: &str,
    global_idx: usize,
    mem_offset: i64,
    debug_fn: &DebugFunction,
    stack_buf: &JsValue,
    prog_buf: &JsValue,
    frame_sp: u32,
    bkpt_idx: usize,
    is_caller_frame: bool,
    kind: &TypeKind,
    byte_size: u32,
) -> Option<String> {
    let frame_entry = match debug_fn.layout.iter().find(|e| {
        let location_match = e.location == WasmLocation::Global(global_idx);
        if is_caller_frame {
            location_match
        } else {
            location_match && e.lifetime.contains(&bkpt_idx)
        }
    }) {
        Some(e) => e,
        None => {
            crate::log!(
                "[host] '{}': global {} not saved for bkpt {} (layout: {:?})",
                name, global_idx, bkpt_idx,
                debug_fn.layout.iter().map(|e| format!("{:?}", e.location)).collect::<Vec<_>>()
            );
            return None;
        }
    };

    // The global's value (C stack pointer) is saved as I32 at frame_sp + entry.offset.
    let base_addr = js_sys::Uint32Array::new_with_byte_offset_and_length(
        stack_buf,
        frame_sp + frame_entry.offset as u32,
        1,
    )
    .get_index(0);

    let mem_addr = (base_addr as i64 + mem_offset) as u32;
    crate::log!(
        "[host] '{}': linear mem addr = {} + {} = {}",
        name, base_addr, mem_offset, mem_addr
    );

    let wasm_ty = c_val_type(kind, byte_size);
    Some(format_value(prog_buf, mem_addr, wasm_ty, kind))
}

/// Try to build a [Variable] for one `DW_TAG_variable` / `DW_TAG_formal_parameter` entry.
fn make_variable<R: Reader>(
    dwarf: &gimli::Dwarf<R>,
    unit: &gimli::Unit<R>,
    entry: &gimli::DebuggingInformationEntry<R>,
    debug_fn: &DebugFunction,
    stack_buf: &JsValue,
    prog_buf: &JsValue,
    frame_sp: u32,
    bkpt_idx: usize,
    frame_base_global: Option<usize>,
    is_caller_frame: bool,
) -> Option<Variable> {
    // --- name ---
    let name_attr = entry.attr(gimli::DW_AT_name)?;
    let name_str = dwarf.attr_string(unit, name_attr.value()).ok()?;
    let name = name_str.to_string_lossy().ok()?.into_owned();

    // --- location ---
    let loc_attr = match entry.attr(gimli::DW_AT_location) {
        Some(a) => a,
        None => {
            crate::log!("[host] '{}': no DW_AT_location", name);
            return None;
        }
    };
    let var_loc = match loc_attr.value() {
        gimli::AttributeValue::Exprloc(expr) => match wasm_loc_from_expr(expr, unit.encoding()) {
            Some(l) => l,
            None => {
                crate::log!("[host] '{}': unrecognized location expression", name);
                return None;
            }
        },
        other => {
            crate::log!("[host] '{}': location attr is not Exprloc: {:?}", name, other);
            return None;
        }
    };

    // --- type name, interpretation, and byte size ---
    let (ty_name, mut kind, mut byte_size) = entry
        .attr(gimli::DW_AT_type)
        .map(|a| read_type(dwarf, unit, a.value(), 8))
        .unwrap_or_else(|| ("?".to_string(), TypeKind::Other, 4));

    // If DWARF gave us Other (missing/wrong type ref or encoding), infer float from type name
    // so that e.g. "float j = 10.4" displays correctly.
    if kind == TypeKind::Other {
        let name_lower = ty_name.to_lowercase();
        if name_lower == "float" || name_lower.starts_with("float ") {
            kind = TypeKind::Float;
            byte_size = 4;
        } else if name_lower == "double" || name_lower.starts_with("double ") {
            kind = TypeKind::Float;
            byte_size = 8;
        }
    }

    // --- read the value based on location kind ---
    // Use the variable's type from DWARF (kind + byte_size) to interpret the value,
    // not the layout's stored type, so floats and other types display correctly
    // even if the layout type was wrong or mismatched.
    let read_ty = c_val_type(&kind, byte_size);
    let value = match var_loc {
        VarLoc::Wasm(WasmLocation::Local(local_idx)) => {
            let frame_entry = match debug_fn.layout.iter().find(|e| {
                let location_match = e.location == WasmLocation::Local(local_idx);
                if is_caller_frame {
                    location_match
                } else {
                    location_match && e.lifetime.contains(&bkpt_idx)
                }
            }) {
                Some(e) => e,
                None => {
                    crate::log!(
                        "[host] '{}': local {} not saved for bkpt {}",
                        name, local_idx, bkpt_idx
                    );
                    return None;
                }
            };
            format_value(stack_buf, frame_sp + frame_entry.offset as u32, read_ty, &kind)
        }

        VarLoc::Wasm(WasmLocation::Global(global_idx)) => {
            let frame_entry = match debug_fn.layout.iter().find(|e| {
                let location_match = e.location == WasmLocation::Global(global_idx);
                if is_caller_frame {
                    location_match
                } else {
                    location_match && e.lifetime.contains(&bkpt_idx)
                }
            }) {
                Some(e) => e,
                None => {
                    crate::log!("[host] '{}': global {} not saved for bkpt {}", name, global_idx, bkpt_idx);
                    return None;
                }
            };
            format_value(stack_buf, frame_sp + frame_entry.offset as u32, read_ty, &kind)
        }

        VarLoc::FrameRelative(frame_offset) => {
            let global_idx = match frame_base_global {
                Some(g) => g,
                None => {
                    crate::log!("[host] '{}': FrameRelative but no frame_base_global", name);
                    return None;
                }
            };
            read_from_linear_memory(
                &name, global_idx, frame_offset, debug_fn,
                stack_buf, prog_buf, frame_sp, bkpt_idx, is_caller_frame, &kind, byte_size,
            )?
        }

        VarLoc::GlobalPlusOffset { global_idx, offset } => {
            read_from_linear_memory(
                &name, global_idx, offset as i64, debug_fn,
                stack_buf, prog_buf, frame_sp, bkpt_idx, is_caller_frame, &kind, byte_size,
            )?
        }

        VarLoc::Wasm(WasmLocation::Operand(_)) => {
            crate::log!("[host] '{}': operand-stack locations not yet supported", name);
            return None;
        }
    };

    Some(Variable { name, ty: ty_name, value })
}

/// Decode a DWARF location expression into a [VarLoc].
///
/// Handles:
/// - Single WASM ops: `DW_OP_WASM_location` (local/global/stack)
/// - Frame-relative: `DW_OP_fbreg N` → `FrameRelative(N)`
/// - Two-op global+offset: `DW_OP_WASM_location global N; DW_OP_plus_uconst M` → `GlobalPlusOffset`
/// - Standalone `DW_OP_plus_uconst N` → `FrameRelative(N)` (implicit frame base)
fn wasm_loc_from_expr<R: Reader>(
    expr: gimli::Expression<R>,
    encoding: gimli::Encoding,
) -> Option<VarLoc> {
    let mut ops = expr.operations(encoding);
    let first = ops.next().ok()??;
    match first {
        gimli::Operation::WasmLocal { index } => Some(VarLoc::Wasm(WasmLocation::Local(index as usize))),
        gimli::Operation::WasmGlobal { index } => {
            // Check for a following DW_OP_plus_uconst (two-op frame+offset expression).
            match ops.next().ok()? {
                Some(gimli::Operation::PlusConstant { value }) => {
                    Some(VarLoc::GlobalPlusOffset { global_idx: index as usize, offset: value })
                }
                _ => Some(VarLoc::Wasm(WasmLocation::Global(index as usize))),
            }
        }
        gimli::Operation::WasmStack { index } => Some(VarLoc::Wasm(WasmLocation::Operand(index as usize))),
        // DW_OP_fbreg N: frame-relative offset.
        gimli::Operation::FrameOffset { offset } => Some(VarLoc::FrameRelative(offset)),
        // DW_OP_plus_uconst N alone: treated as frame-relative (implicit frame base).
        gimli::Operation::PlusConstant { value } => Some(VarLoc::FrameRelative(value as i64)),
        op => {
            crate::log!("[host] unrecognized location op: {:?}", op);
            None
        }
    }
}

/// Recursively follow a DWARF type reference to extract (name, TypeKind, byte_size).
/// `depth` prevents infinite loops through pointer/typedef chains.
fn read_type<R: Reader>(
    dwarf: &gimli::Dwarf<R>,
    unit: &gimli::Unit<R>,
    type_ref: gimli::AttributeValue<R>,
    depth: u8,
) -> (String, TypeKind, u32) {
    if depth == 0 {
        return ("?".to_string(), TypeKind::Other, 4);
    }

    let offset = match type_ref {
        gimli::AttributeValue::UnitRef(o) => o,
        _ => return ("?".to_string(), TypeKind::Other, 4),
    };

    let Ok(entry) = unit.entry(offset) else {
        return ("?".to_string(), TypeKind::Other, 4);
    };

    let tag = entry.tag();

    // Helper: read DW_AT_name as a String.
    let get_name = || -> String {
        entry
            .attr(gimli::DW_AT_name)
            .and_then(|a| dwarf.attr_string(unit, a.value()).ok())
            .and_then(|s| s.to_string_lossy().ok().map(|c| c.into_owned()))
            .unwrap_or_default()
    };

    // Helper: read DW_AT_byte_size.
    let get_byte_size = || -> u32 {
        entry
            .attr(gimli::DW_AT_byte_size)
            .and_then(|a| a.value().udata_value())
            .unwrap_or(4) as u32
    };

    // Helper: follow DW_AT_type to an inner AttributeValue.
    let inner_attr = || -> Option<gimli::AttributeValue<R>> {
        Some(entry.attr(gimli::DW_AT_type)?.value())
    };

    match tag {
        gimli::DW_TAG_base_type => {
            let name = get_name();
            let byte_size = get_byte_size();
            let kind = entry
                .attr(gimli::DW_AT_encoding)
                .and_then(|a| a.value().udata_value())
                .map(|v| match v as u16 {
                    5 | 6 => TypeKind::Signed,   // DW_ATE_signed / DW_ATE_signed_char
                    7 | 8 => TypeKind::Unsigned,  // DW_ATE_unsigned / DW_ATE_unsigned_char
                    4 => TypeKind::Float,          // DW_ATE_float
                    2 => TypeKind::Bool,           // DW_ATE_boolean
                    _ => TypeKind::Unsigned,
                })
                .unwrap_or_else(|| {
                    // Encoding missing: infer from type name so float/double display correctly
                    let n = name.to_lowercase();
                    if n == "float" || n == "double" {
                        TypeKind::Float
                    } else {
                        TypeKind::Unsigned
                    }
                });
            (name, kind, byte_size)
        }

        gimli::DW_TAG_pointer_type => {
            let inner_name = inner_attr()
                .map(|a| read_type(dwarf, unit, a, depth - 1).0)
                .unwrap_or_else(|| "void".to_string());
            (format!("{}*", inner_name), TypeKind::Pointer, 4)
        }

        // typedef / const / volatile: unwrap to the inner type.
        gimli::DW_TAG_typedef | gimli::DW_TAG_const_type | gimli::DW_TAG_volatile_type => {
            match inner_attr() {
                Some(inner) => {
                    let (inner_name, kind, byte_size) = read_type(dwarf, unit, inner, depth - 1);
                    let name = if tag == gimli::DW_TAG_typedef {
                        let n = get_name();
                        if n.is_empty() { inner_name } else { n }
                    } else {
                        inner_name
                    };
                    (name, kind, byte_size)
                }
                None => (get_name(), TypeKind::Other, 4),
            }
        }

        _ => {
            // struct, union, array, enum, …
            let name = get_name();
            let name = if name.is_empty() { format!("<{}>", tag) } else { name };
            let byte_size = get_byte_size();
            (name, TypeKind::Other, byte_size)
        }
    }
}

/// Map a C type's kind and byte size to the WASM ValType used to read it from memory.
fn c_val_type(kind: &TypeKind, byte_size: u32) -> wasmparser::ValType {
    use wasmparser::ValType;
    match kind {
        TypeKind::Float => if byte_size <= 4 { ValType::F32 } else { ValType::F64 },
        TypeKind::Pointer => ValType::I32,
        _ => if byte_size <= 4 { ValType::I32 } else { ValType::I64 },
    }
}

/// Read a value from the saved stack frame and format it as a string.
fn format_value(
    stack_buf: &JsValue,
    byte_offset: u32,
    wasm_ty: wasmparser::ValType,
    kind: &TypeKind,
) -> String {
    use wasmparser::ValType;
    match wasm_ty {
        ValType::I32 => {
            let v =
                js_sys::Int32Array::new_with_byte_offset_and_length(stack_buf, byte_offset, 1)
                    .get_index(0);
            match kind {
                TypeKind::Unsigned => format!("{}", v as u32),
                TypeKind::Bool => {
                    if v != 0 { "true".to_string() } else { "false".to_string() }
                }
                TypeKind::Pointer => format!("0x{:08x}", v as u32),
                _ => format!("{}", v), // signed by default
            }
        }
        ValType::I64 => {
            // Read as two consecutive i32 words (little-endian).
            let view =
                js_sys::Int32Array::new_with_byte_offset_and_length(stack_buf, byte_offset, 2);
            let lo = view.get_index(0) as u64;
            let hi = view.get_index(1) as u64;
            let v = (hi << 32) | lo;
            match kind {
                TypeKind::Unsigned => format!("{}", v),
                _ => format!("{}", v as i64),
            }
        }
        ValType::F32 => {
            let v =
                js_sys::Float32Array::new_with_byte_offset_and_length(stack_buf, byte_offset, 1)
                    .get_index(0);
            format!("{}", v)
        }
        ValType::F64 => {
            let v =
                js_sys::Float64Array::new_with_byte_offset_and_length(stack_buf, byte_offset, 1)
                    .get_index(0);
            format!("{}", v)
        }
        _ => String::new(),
    }
}
