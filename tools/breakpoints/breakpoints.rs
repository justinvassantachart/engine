use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::env;
use std::fs;
use std::process;

use engine::debug::instrument::{instrument_wasm, InstrumenterResult};
use wasmparser::{Parser, Payload};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: breakpoints <path-to-wasm>");
        process::exit(1);
    }

    let path = &args[1];
    let wasm_bytes = fs::read(path).unwrap_or_else(|e| {
        eprintln!("Failed to read {}: {}", path, e);
        process::exit(1);
    });

    let InstrumenterResult {
        wasm: instrumented,
        ref info,
    } = match instrument_wasm(&wasm_bytes) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to parse/instrument WASM: {}", e);
            process::exit(1);
        }
    };

    let locations = &info.locations;
    let files = &info.files;

    println!("=== DWARF locations ({}) ===", locations.len());
    for (i, loc) in locations.iter().enumerate() {
        let fname = files
            .get(loc.file)
            .map(|s| s.as_str())
            .unwrap_or("?");
        println!(
            "  [{}] {}:{}:{} @ 0x{:x}",
            i + 1,
            fname,
            loc.line,
            loc.col,
            loc.address
        );
    }
    println!();

    let orig_len = wasm_bytes.len() as i64;
    let inst_len = instrumented.len() as i64;
    println!(
        "Binary: {} -> {} bytes ({:+})\n",
        orig_len,
        inst_len,
        inst_len - orig_len
    );

    let original_funcs = disassemble_functions(&wasm_bytes);
    let instrumented_funcs = disassemble_functions(&instrumented);
    let bkpt_fn = find_breakpoint_fn(&instrumented_funcs);

    let mut total_bkpts = 0;
    let mut bkpt_indices: Vec<i32> = Vec::new();

    let max_funcs = original_funcs.len().max(instrumented_funcs.len());
    for i in 0..max_funcs {
        let orig = original_funcs.get(i).map(|v| v.as_slice()).unwrap_or(&[]);
        let inst = instrumented_funcs
            .get(i)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        if orig.len() == inst.len() {
            continue;
        }
        for bkpt in extract_breakpoints(inst, bkpt_fn) {
            bkpt_indices.push(bkpt.index);
            total_bkpts += 1;
        }
    }

    // Group breakpoints by file, then collect lines with breakpoints
    // file_index -> sorted set of lines
    let mut by_file: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for idx in &bkpt_indices {
        let loc_idx = (*idx - 1) as usize;
        if let Some(loc) = locations.get(loc_idx) {
            by_file.entry(loc.file).or_default().insert(loc.line);
        }
    }

    // Also collect ALL dwarf lines per file (including ones that may not have been injected)
    let mut all_lines_by_file: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for loc in &locations {
        all_lines_by_file
            .entry(loc.file)
            .or_default()
            .insert(loc.line);
    }

    println!(
        "{} breakpoints injected across {} functions\n",
        total_bkpts,
        by_file.len().max(1) // at least show something meaningful
    );

    // Build a map of (file, line) -> count of breakpoints on that line
    let mut line_counts: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for idx in &bkpt_indices {
        let loc_idx = (*idx - 1) as usize;
        if let Some(loc) = locations.get(loc_idx) {
            *line_counts.entry((loc.file, loc.line)).or_default() += 1;
        }
    }

    for (file_idx, breakpoint_lines) in &by_file {
        let fname = files
            .get(*file_idx)
            .map(|s| s.as_str())
            .unwrap_or("?");

        let lines: Vec<_> = breakpoint_lines.iter().collect();
        println!("{}:", fname);
        for (i, &&line) in lines.iter().enumerate() {
            if i > 0 {
                let prev = *lines[i - 1];
                if line > prev + 1 {
                    println!("    ...");
                }
            }
            let count = line_counts.get(&(*file_idx, line)).copied().unwrap_or(1);
            if count > 1 {
                println!("  * line {} (x{})", line, count);
            } else {
                println!("  * line {}", line);
            }
        }
        println!();
    }

    // Check for DWARF locations that weren't injected
    let injected_set: BTreeSet<i32> = bkpt_indices.iter().copied().collect();
    let mut missed = 0;
    for (i, loc) in locations.iter().enumerate() {
        if !injected_set.contains(&((i + 1) as i32)) {
            if missed == 0 {
                println!("Missed DWARF locations (no breakpoint injected):");
            }
            let fname = files
                .get(loc.file)
                .map(|s| s.as_str())
                .unwrap_or("?");
            println!(
                "  [{}] {}:{}:{} @ 0x{:x}",
                i + 1,
                fname,
                loc.line,
                loc.col,
                loc.address
            );
            missed += 1;
        }
    }
    if missed > 0 {
        println!();
    }
}

struct BreakpointHit {
    index: i32,
}

fn find_breakpoint_fn(funcs: &[Vec<String>]) -> Option<u32> {
    let mut call_counts: HashMap<u32, u32> = HashMap::new();

    for instrs in funcs {
        for pair in instrs.windows(2) {
            if pair[0].starts_with("I32Const") {
                if let Some(idx) = parse_call_index(&pair[1]) {
                    *call_counts.entry(idx).or_default() += 1;
                }
            }
        }
    }

    call_counts
        .into_iter()
        .max_by_key(|&(_, count)| count)
        .map(|(idx, _)| idx)
}

fn parse_call_index(s: &str) -> Option<u32> {
    let prefix = "Call { function_index: ";
    let start = s.find(prefix)? + prefix.len();
    let end = s[start..].find(' ').unwrap_or(s.len() - start - 1) + start;
    s[start..end].trim_end_matches('}').trim().parse().ok()
}

fn parse_i32_const(s: &str) -> Option<i32> {
    let prefix = "I32Const { value: ";
    let start = s.find(prefix)? + prefix.len();
    let end = s[start..].find(' ').unwrap_or(s.len() - start - 1) + start;
    s[start..end].trim_end_matches('}').trim().parse().ok()
}

fn extract_breakpoints(instrs: &[String], bkpt_fn: Option<u32>) -> Vec<BreakpointHit> {
    let bkpt_fn = match bkpt_fn {
        Some(f) => f,
        None => return vec![],
    };

    let mut result = Vec::new();
    let mut i = 0;
    while i + 1 < instrs.len() {
        if let (Some(val), Some(call_idx)) = (
            parse_i32_const(&instrs[i]),
            parse_call_index(&instrs[i + 1]),
        ) {
            if call_idx == bkpt_fn {
                result.push(BreakpointHit { index: val });
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    result
}

fn disassemble_functions(wasm_bytes: &[u8]) -> Vec<Vec<String>> {
    let mut functions = Vec::new();

    for payload in Parser::new(0).parse_all(wasm_bytes) {
        let payload = match payload {
            Ok(p) => p,
            Err(_) => continue,
        };

        if let Payload::CodeSectionEntry(body) = payload {
            let mut instrs = Vec::new();
            let reader = match body.get_operators_reader() {
                Ok(r) => r,
                Err(_) => {
                    functions.push(instrs);
                    continue;
                }
            };

            for op in reader {
                match op {
                    Ok(op) => instrs.push(format!("{:?}", op)),
                    Err(e) => instrs.push(format!("<error: {}>", e)),
                }
            }
            functions.push(instrs);
        }
    }

    functions
}
