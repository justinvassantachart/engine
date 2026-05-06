//! WASM instrumentation for debugging.
pub mod encoder;
pub mod function;
pub use encoder::*;
pub use function::*;

use crate::types::DebugInfo;
use anyhow::Result;
use wasm_encoder::reencode;

pub type Error = anyhow::Error;
pub type InstrError = reencode::Error<Error>;
pub type InstrResult<T = ()> = std::result::Result<T, InstrError>;

pub struct InstrumenterResult {
    pub wasm: Vec<u8>,
    pub info: DebugInfo,
}

/// Instrument a WASM binary to support debugging
pub fn instrument_wasm(wasm: &[u8]) -> Result<InstrumenterResult> {
    let mut instrumenter = encoder::Instrumenter::new(wasm)?;
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
        info: instrumenter.finish(),
    })
}
