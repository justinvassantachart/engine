use console_error_panic_hook;
use std::path::PathBuf;
use wasm_bindgen::prelude::*;
use wasmer_wasix::virtual_fs::{AsyncWriteExt, FileSystem, create_dir_all, mem_fs};
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent};

use crate::types::{FsNode, WorkerOut, WorkerStart};

mod debuggee;
mod execution;
mod io;
mod runtime;

use execution::Execution;

// ============================================================================
// Helpers
// ============================================================================

async fn create_user_fs(node: FsNode) -> Result<mem_fs::FileSystem, std::io::Error> {
    let fs = mem_fs::FileSystem::default();
    create_user_fs_rec(&fs, &PathBuf::from("/"), &node).await?;
    Ok(fs)
}

async fn create_user_fs_rec(
    fs: &mem_fs::FileSystem,
    base_path: &PathBuf,
    node: &FsNode,
) -> Result<(), std::io::Error> {
    match node {
        FsNode::File(contents) => {
            let mut file = fs
                .new_open_options()
                .create(true)
                .write(true)
                .open(base_path)?;
            file.write_all(contents.as_bytes())
                .await
                .expect("Failed to write injected file");
            file.flush().await.expect("Flushed file")
        }
        FsNode::Dir(children) => {
            create_dir_all(fs, base_path)?;
            for (name, child_node) in children {
                let mut child_path = base_path.clone();
                child_path.push(name);
                Box::pin(create_user_fs_rec(fs, &child_path, child_node)).await?;
            }
        }
    }
    Ok(())
}

fn collect_sources(node: &FsNode, base_path: &PathBuf, sources: &mut Vec<String>) {
    match node {
        FsNode::File(_) => {
            if let Some(ext) = base_path.extension().and_then(|ext| ext.to_str()) {
                let is_source = matches!(
                    ext.to_ascii_lowercase().as_str(),
                    "c" | "cc" | "cp" | "cpp" | "cxx" | "c++"
                );
                if is_source {
                    sources.push(base_path.to_string_lossy().to_string());
                }
            }
        }
        FsNode::Dir(children) => {
            collect_dir_sources(children, base_path, sources);
        }
    }
}

fn collect_dir_sources(
    children: &std::collections::HashMap<String, FsNode>,
    base_path: &PathBuf,
    sources: &mut Vec<String>,
) {
    for (name, child_node) in children {
        let mut child_path = base_path.clone();
        child_path.push(name);
        collect_sources(child_node, &child_path, sources);
    }
}

// ============================================================================
// Worker
// ============================================================================

async fn start(msg: WorkerStart) {
    let mut sources = Vec::new();
    collect_dir_sources(&msg.fs, &PathBuf::from("/"), &mut sources);
    sources.sort();

    assert!(
        !sources.is_empty(),
        "No C/C++ source files found in provided filesystem"
    );

    let fs = create_user_fs(FsNode::Dir(msg.fs))
        .await
        .expect("created user files filesystem");

    let exec = Execution::new(msg.stdin_buffer);

    // Build clang args, conditional on is_debug
    let mut clang_args = vec![
        "-cc1",
        "-triple",
        "wasm32-wasip1",
        "-Werror",
        "-emit-obj",
        "-disable-free",
        "-isysroot",
        "/",
        "-internal-isystem",
        "/include/c++/v1",
        "-internal-isystem",
        "/include",
        "-internal-isystem",
        "/include/wasm32-wasip1",
        "-ferror-limit",
        "4",
        "-fcolor-diagnostics",
        "-x",
        "c++",
        "-std=c++23",
        "-o",
        "/main.o",
    ];

    if msg.is_debug {
        clang_args.push("-O0");
        // because of the -cc1 flag
        clang_args.push("-debug-info-kind=standalone");
        clang_args.push("-dwarf-version=5");
    }

    for source in &sources {
        clang_args.push(source);
    }

    exec.step("clang")
        // from @yowasp
        .binary("https://fabioibanez.github.io/website/llvm.core.wasm")
        .sysroot("https://fabioibanez.github.io/website/llvm-resources.tar.gz")
        .fs(Box::new(fs))
        .args(&clang_args)
        .run()
        .await
        .expect("Compilation succeeded");

    exec.step("wasm-ld")
        .binary("https://fabioibanez.github.io/website/llvm.core.wasm")
        .args(&[
            "--export-dynamic",
            "-z",
            "stack-size=1048576",
            "-L/lib/wasm32-wasip1",
            "/lib/wasm32-wasip1/crt1.o",
            "/main.o",
            "-lc++",
            "-lc++abi",
            "/lib/wasm32-unknown-wasip1/libclang_rt.builtins.a",
            "-lc",
            "-o",
            "/main.wasm",
        ])
        .run()
        .await
        .expect("Linking succeeded");

    exec.step("main")
        .binary("/main.wasm")
        .debug(msg.is_debug)
        .run()
        .await
        .expect("Running succeeded");

    // s as dasdadsaadsadsa asadsdsa

    // Send Stop message on successful completion
    WorkerOut::Stop.send();
}

#[wasm_bindgen]
pub fn main() {
    console_error_panic_hook::set_once();
    let scope = DedicatedWorkerGlobalScope::from(JsValue::from(js_sys::global()));

    // Function that gets called when the worker receives a message
    let onmessage = Closure::wrap(Box::new(move |msg: MessageEvent| {
        let message: WorkerStart = serde_wasm_bindgen::from_value(msg.data()).expect("");
        // rust-ism: spawn_local is used to run the start function in a new thread
        wasm_bindgen_futures::spawn_local(start(message));
    }) as Box<dyn Fn(MessageEvent)>);
    scope.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    onmessage.forget();

    // The worker must send a message to indicate that it's ready to receive messages.
    WorkerOut::Ready.send();
}
