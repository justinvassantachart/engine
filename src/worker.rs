use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tsify::Tsify; // takes the Rust types and generates TypeScript definitions
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasmer::{Instance, Module, Store, Value, imports};
use wasmer_wasix::WasiEnv;
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent};

#[derive(Tsify, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FsNode {
    File(String),
    Dir(HashMap<String, FsNode>),
}

#[derive(Tsify, Serialize, Deserialize)]
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

async fn get_url(url: &str) -> Result<Vec<u8>, JsValue> {
    // use websys to get the binary from a link
    web_sys::console::log_1(&"Fetching  binary...".into());

    // https://docs.rs/web-sys/latest/web_sys/struct.DedicatedWorkerGlobalScope.html#method.fetch_with_str
    let scope = DedicatedWorkerGlobalScope::from(JsValue::from(js_sys::global()));
    let resp_value = JsFuture::from(scope.fetch_with_str(&url)).await?;
    let resp = resp_value.dyn_into::<web_sys::Response>()?;
    let array_buffer = JsFuture::from(resp.array_buffer()?).await?;
    let uint8_array = js_sys::Uint8Array::new(&array_buffer);
    let bytes = uint8_array.to_vec();
    Ok(bytes)
}

async fn start(msg: WorkerStart) {
    web_sys::console::log_2(
        &"Started!".into(),
        // the following line serializes the Fs structure back to JS for logging
        // it returns a Result<JsValue>, so we unwrap it with expect
        &serde_wasm_bindgen::to_value(&msg.fs).expect("serialization worked"),
    );

    /* Fetch a binary using web sys */
    let binary_url = "https://runno.dev/langs/clang.wasm";
    let sysroot_url = "https://runno.dev/langs/clang-fs.tar.gz";
    let binary = get_url(binary_url).await.expect("Failed to fetch binary");
    let sysroot = get_url(sysroot_url).await.expect("Failed to fetch sysroot");

    let mut store = Store::default();

    // Need to pass bytes to Module::from_binary
    // https://wasmerio.github.io/wasmer/crates/doc/wasmer/struct.Module.html
    let clang_binary = Module::from_binary(&store, &binary)
        .expect("Failed to create module from binary");

    // Set up WASI environment
    WasiEnv::builder("clang").instantiate(clang_binary, &mut store);
}

#[wasm_bindgen]
pub fn main() {
    web_sys::console::log_1(&"worker starting".into());

    let scope = DedicatedWorkerGlobalScope::from(JsValue::from(js_sys::global()));

    // Function that gets called when the worker receives a message
    let onmessage = Closure::wrap(Box::new(move |msg: MessageEvent| {
        web_sys::console::log_1(&"got message".into());
        let message: WorkerStart = serde_wasm_bindgen::from_value(msg.data()).expect("");
        start(message);
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
