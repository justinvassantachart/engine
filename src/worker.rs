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
use wasmer_wasix::virtual_fs::create_dir_all;
use wasmer_wasix::virtual_fs::{AsyncWriteExt, FileSystem, mem_fs};
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent};

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
            fs.create_dir(&abs_path)
                .expect("Failed to create directory")
        } else {
            create_dir_all(&fs, &abs_path).expect("Failed to create parent directories");
            let mut file = fs
                .new_open_options()
                .create(true)
                .write(true)
                .open(&abs_path)
                .expect("Failed to create file");

            let mut contents = Vec::new();
            entry.read_to_end(&mut contents)?;
            file.write(&contents).await.expect("Failed to write file");
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
            file.write_all(contents.as_bytes())
                .await
                .expect("Failed to write injected file");
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
        futures::try_join!(fetch_bytes(CLANG_WASM_URL), fetch_bytes(CLANG_SYSROOT_URL),)
            .expect("Failed to fetch binaries");

    let mut store = Store::default();

    // Need to pass bytes to Module::from_binary
    // https://wasmerio.github.io/wasmer/crates/doc/wasmer/struct.Module.html
    let clang_binary =
        Module::from_binary(&store, &clang_wasm_bytes).expect("Failed to compile clang module");

    let fs = extract_tar_gz(sysroot_bytes)
        .await
        .expect("Failed to extract sysroot into mem_fs");

    inject_files(&fs, &PathBuf::from("/"), &FsNode::Dir(msg.fs))
        .await
        .expect("Failed to inject user files into mem_fs");

    // Must call instatiate after writing files
    let (instance, env) = WasiEnv::builder("clang")
        .runtime(runtime::JsRuntime::instance())
        .fs(Box::new(fs)) // Mount the virtual filesystem
        .instantiate(clang_binary, &mut store)
        .expect("Failed to instantiate WASI");

    let start = instance
        .exports
        .get_function("_start")
        .expect("Failed to find _start function");

    start.call(&mut store, &[]).expect("Failed to run _start");
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
