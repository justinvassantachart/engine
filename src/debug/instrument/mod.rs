//! WASM instrumentation for debugging.
pub mod encoder;
pub mod function;
pub use encoder::*;
pub use function::*;

use crate::debug::dwarf::Dwarf;
use crate::types::{DebugInfo, MemoryDescriptor};
use anyhow::Result;
use std::collections::HashMap;
use wasm_encoder::reencode;
use wasmparser::Payload;

pub type Error = anyhow::Error;
pub type InstrError = reencode::Error<Error>;
pub type InstrResult<T = ()> = std::result::Result<T, InstrError>;

fn parse_debug_info(wasm: &[u8]) -> Result<DebugInfo> {
    let mut sections: HashMap<&str, &[u8]> = HashMap::new();
    let mut memory_initial = 0u32;

    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        let payload = payload?;
        match payload {
            Payload::CustomSection(reader) => {
                sections.insert(reader.name(), reader.data());
            }
            Payload::MemorySection(reader) => {
                for mem in reader {
                    let mem = mem?;
                    memory_initial = mem.initial as u32;
                    break;
                }
            }
            _ => {}
        }
    }

    Ok(DebugInfo {
        functions: Vec::new(),
        breakpoints: js_sys::SharedArrayBuffer::new(0),
        memory: MemoryDescriptor::new(memory_initial, 16 * memory_initial),
        stack: MemoryDescriptor::new(16, 16),
        dwarf: Dwarf::from_sections(&sections)?,
    })
}

pub struct InstrumenterResult {
    pub wasm: Vec<u8>,
    pub info: DebugInfo,
}

/// Instrument a WASM binary to support debugging
pub fn instrument_wasm(wasm: &[u8]) -> Result<InstrumenterResult> {
    let mut info = parse_debug_info(wasm)?;
    let mut instrumenter = encoder::Instrumenter::new(&mut info);
    let mut module = wasm_encoder::Module::new();
    reencode::utils::parse_core_module(
        &mut instrumenter,
        &mut module,
        wasmparser::Parser::new(0),
        wasm,
    )
    .map_err(|e| anyhow::anyhow!("Failed to reencode WASM: {:?}", e))?;
    Ok(InstrumenterResult {
        wasm: module.finish(),
        info,
    })
}
