use console_error_panic_hook;
use std::path::PathBuf;
use wasm_bindgen::prelude::*;
use wasmer_wasix::virtual_fs::{AsyncWriteExt, FileSystem, create_dir_all, mem_fs};
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent};

use crate::execution::Execution;
use crate::types::*;
use crate::debug::*;

mod debug;
mod execution;
mod io;
mod runtime;
mod types;

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
            web_sys::console::log_1(&format!("Injecting file at {:?}", base_path).into());
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

// ============================================================================
// Worker
// ============================================================================

async fn start(msg: WorkerStart) {
    web_sys::console::log_1(&format!("Started! {:?}", msg).into());

    let fs = create_user_fs(FsNode::Dir(msg.fs))
        .await
        .expect("created user files filesystem");

    let mut exec = Execution::new(msg.stdin_buffer);

    // Build clang args, conditional on is_debug
    let mut clang_args = vec![
        "-cc1",
        "-Werror",
        "-emit-obj",
        "-disable-free",
        "-isysroot",
        "/sys",
        "-internal-isystem",
        "/sys/include/c++/v1",
        "-internal-isystem",
        "/sys/include",
        "-internal-isystem",
        "/sys/lib/clang/8.0.1/include",
        "-ferror-limit",
        "4",
        "-fmessage-length",
        "80",
        "-fcolor-diagnostics",
        "-x",
        "c++",
        "/main.c",
    ];

    if msg.is_debug {
        clang_args.insert(1, "-g");
        clang_args.insert(2, "-O0");
    }

    exec.step("clang")
        .binary("https://runno.dev/langs/clang.wasm")
        .sysroot("https://runno.dev/langs/clang-fs.tar.gz")
        .fs(Box::new(fs))
        .args(&clang_args)
        .run()
        .await
        .expect("Compilation succeeded");

    // TODO: instrument the binary for debugging
    if msg.is_debug {}

    exec.step("wasm-ld")
        .binary("https://runno.dev/langs/wasm-ld.wasm")
        .args(&[
            "--no-threads",
            "--export-dynamic",
            "-z",
            "stack-size=1048576",
            "-L/sys/lib/wasm32-wasi",
            "/sys/lib/wasm32-wasi/crt1.o",
            "/main.o",
            "-lc",
            "-lc++",
            "-lc++abi",
            "-o",
            "/main.wasm",
        ])
        .run()
        .await
        .expect("Linking succeeded");

    exec.step("main")
        .binary("/main.wasm")
        .run()
        .await
        .expect("Running succeeded");

    // Print out the filesystem toplevel for debugging
    let root = exec
        .fs
        .read_dir(PathBuf::from("/").as_path())
        .expect("Read root dir");

    for entry in root {
        web_sys::console::log_1(&format!("FS Entry: {:?}", entry).into());
    }

    WorkerOut::Stop.send();
}

#[wasm_bindgen]
pub fn main() {
    console_error_panic_hook::set_once();
    web_sys::console::log_1(&"worker starting".into());

    let scope = DedicatedWorkerGlobalScope::from(JsValue::from(js_sys::global()));

    // Function that gets called when the worker receives a message
    let onmessage = Closure::wrap(Box::new(move |msg: MessageEvent| {
        web_sys::console::log_1(&"got message".into());
        let message: WorkerStart = serde_wasm_bindgen::from_value(msg.data()).expect("");
        // rust-ism: spawn_local is used to run the start function in a new thread
        wasm_bindgen_futures::spawn_local(start(message));
    }) as Box<dyn Fn(MessageEvent)>);
    scope.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    onmessage.forget();

    // The worker must send a message to indicate that it's ready to receive messages.
    WorkerOut::Ready.send();
}
