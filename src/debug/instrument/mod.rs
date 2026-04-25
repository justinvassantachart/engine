//! WASM instrumentation for debugging.
pub mod encoder;
pub mod function;
pub use encoder::*;
pub use function::*;

use crate::debug::dwarf::Dwarf;
use crate::types::{BP_PREFIX_BYTES, DebugFunction, DebugInfo, MemoryDescriptor};
use crate::util::weak_error;
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

    let dwarf = Dwarf::from_sections(&sections)?;
    let nlocs = dwarf.locations().count();

    Ok(DebugInfo {
        functions: parse_debug_functions(&dwarf),
        breakpoints: js_sys::SharedArrayBuffer::new((BP_PREFIX_BYTES + nlocs) as u32),
        memory: MemoryDescriptor::new(memory_initial, 16 * memory_initial),
        stack: MemoryDescriptor::new(16, 16),
        dwarf,
    })
}

fn parse_debug_functions(dwarf: &Dwarf) -> Vec<DebugFunction> {
    let mut fns = dwarf
        .units()
        .iter()
        .flat_map(|unit| {
            let Some(root) = weak_error!(unit.root(dwarf)) else {
                return Vec::new();
            };

            root.collect_children(|child| {
                if child.tag() != gimli::DW_TAG_subprogram {
                    return None;
                }

                let Some((low_pc, high_pc)) = child.addr_range() else {
                    return None;
                };

                Some(DebugFunction {
                    low_pc,
                    high_pc,
                    die_ref: child.die_ref(),
                    size: 0,
                    layout: Vec::default(),
                })
            })
        })
        .collect::<Vec<DebugFunction>>();
    fns.sort_by_key(|f| f.low_pc);
    fns
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
