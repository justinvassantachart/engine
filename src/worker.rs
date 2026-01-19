use console_error_panic_hook;
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use tar::Archive;
use tsify::Tsify;
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_futures::JsFuture;
use wasmer::{Module, Store};
use wasmer_wasix::WasiEnv;
use wasmer_wasix::virtual_fs::{AsyncWriteExt, FileSystem, mem_fs};
use wasmer_wasix::virtual_fs::{OpenOptionsConfig, create_dir_all};
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent};

use wasmer_wasix::{
    Pipe,
    runners::wasi::{RuntimeOrEngine, WasiRunner},
};

use crate::console::ConsoleFile;

mod console;
mod execution;
mod runtime;

const CLANG_WASM_URL: &str = "https://runno.dev/langs/clang.wasm";
const CLANG_SYSROOT_URL: &str = "https://runno.dev/langs/clang-fs.tar.gz";

// ============================================================================
// Types
// ============================================================================

#[derive(Debug, Tsify, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FsNode {
    File(String),
    Dir(HashMap<String, FsNode>),
}

#[derive(Debug, Tsify, Serialize, Deserialize)]
pub struct WorkerStart {
    fs: HashMap<String, FsNode>,
}

#[derive(Tsify, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WorkerOut {
    #[serde(rename = "ready")]
    Ready,
    #[serde(rename = "stdout")]
    Stdout { data: String },
}

// ============================================================================
// Helpers
// ============================================================================

async fn fetch_bytes(url: &str) -> Result<Vec<u8>, JsValue> {
    let scope = DedicatedWorkerGlobalScope::from(JsValue::from(js_sys::global()));
    let response: web_sys::Response = JsFuture::from(scope.fetch_with_str(url))
        .await?
        .dyn_into()?;
    let buffer = JsFuture::from(response.array_buffer()?).await?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

async fn extract_tar_gz(data: Vec<u8>) -> Result<mem_fs::FileSystem, std::io::Error> {
    let decoder = GzDecoder::new(Cursor::new(data));
    let mut archive = Archive::new(decoder);

    let fs = mem_fs::FileSystem::default();

    for entry in archive.entries()? {
        let mut entry = entry?;
        let is_dir = entry.header().entry_type().is_dir();
        let abs_path = PathBuf::from(format!("/{}", entry.path()?.to_string_lossy()));

        if is_dir {
            fs.create_dir(&abs_path).expect("Created directory")
        } else {
            if let Some(parent) = abs_path.parent() {
                create_dir_all(&fs, &parent).expect("Created parent directories");
            }
            let mut file = fs
                .new_open_options()
                .create(true)
                .write(true)
                .open(&abs_path)
                .expect("Created file");

            let mut contents = Vec::new();
            entry.read_to_end(&mut contents)?;
            file.write(&contents).await.expect("Wrote file");
        }
    }
    Ok(fs)
}

async fn inject_files(
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
                Box::pin(inject_files(fs, &child_path, child_node))
                    .await
                    .expect("Failed to create injected children files")
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

    // Parallelizes the two fetches
    let (clang_wasm_bytes, sysroot_bytes) =
        // the sysroot is a tar.gz archive that is the root filesystem for clang 
        futures::try_join!(fetch_bytes(CLANG_WASM_URL), fetch_bytes(CLANG_SYSROOT_URL),)
            .expect("fetched clang wasm and sysroot");

    // this is the wasmer store that holds the state of the module
    let mut store = Store::default();

    // Need to pass bytes to Module::from_binary
    // https://wasmerio.github.io/wasmer/crates/doc/wasmer/struct.Module.html
    let clang_binary =
        Module::from_binary(&store, &clang_wasm_bytes).expect("Succeeded compiling clang module");

    let fs = extract_tar_gz(sysroot_bytes)
        .await
        .expect("extracted sysroot filesystem into mem fs");

    // overlay user files onto the sysroot
    inject_files(&fs, &PathBuf::from("/"), &FsNode::Dir(msg.fs))
        .await
        .expect("injected user files");

    web_sys::console::log_1(&format!("FS: {:?}", fs).into());

    // Must call instatiate after writing files
    // WasiEnv builder to configure the environment
    let mut builder = WasiEnv::builder("clang") // name becomes argv[0]
        .runtime(runtime::JsRuntime::instance())
        // We are going to put fs into a box to satisfy the trait object requirement.
        .fs(Box::new(fs)) // Mount the virtual filesystem. A box is a pointer type in Rust
        .args(&[
            // "--version",
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
            "-O2",
            "-x",
            "c++",
            "/main.c",
        ])
        .stdout(Box::new(ConsoleFile::default()))
        .stderr(Box::new(ConsoleFile::default()));

    // This guy preopens the root directory of the virtual FS to the WASI module
    // it is what allows the module to see the files we put in there
    builder.add_preopen_dir("/").expect("preopen");

    let (instance, env) = builder
        .instantiate(clang_binary, &mut store)
        .expect("Instantiated clang module");

    let start = instance
        .exports
        .get_function("_start")
        .expect("Found _start function");

    start.call(&mut store, &[]).expect("Ran _start function");

    let fs = env.data(&store).fs_root();

    // Print out the filesystem toplevel for debugging
    let root = fs
        .read_dir(PathBuf::from("/").as_path())
        .expect("Read root dir");
    for entry in root {
        web_sys::console::log_1(&format!("FS Entry: {:?}", entry).into());
    }
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
    scope
        .post_message(
            &serde_wasm_bindgen::to_value(&WorkerOut::Ready)
                .expect("serialization worked")
                .into(),
        )
        .expect("posting ready message succeeds!");
}
