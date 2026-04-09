use flate2::read::GzDecoder;
use js_sys::SharedArrayBuffer;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::Arc;
use tar::Archive;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use wasmer::{Module, RuntimeError, Store};
use wasmer_wasix::virtual_fs::AsyncReadExt;
use wasmer_wasix::{
    WasiEnv, WasiEnvBuilder, WasiError, WasiFunctionEnv,
    virtual_fs::{AsyncWriteExt, FileSystem, create_dir_all, mem_fs},
};

use web_sys::DedicatedWorkerGlobalScope;

use crate::debug::WorkerDebugger;
use crate::debug::dwarf_old::parse_debug_info;
use crate::debug::instrument::{InstrumenterResult, instrument_wasm};
use crate::types::{StdoutMode, WorkerOut};

use super::io::{Stdin, Stdout};
use super::runtime::JsRuntime;

use std::fmt::Debug;

pub struct Execution {
    pub fs: mem_fs::FileSystem,
    pub stdin_buffer: js_sys::SharedArrayBuffer,
}

pub struct Step<'a> {
    // exec cannot outlive Step
    exec: &'a Execution,
    builder: WasiEnvBuilder,
    binary: Option<String>,
    sysroot: Option<String>,
    union_fs: Option<Box<dyn FileSystem>>,
    debug: bool,
}

impl Execution {
    pub fn new(stdin_buffer: SharedArrayBuffer) -> Self {
        Self {
            fs: mem_fs::FileSystem::default(),
            stdin_buffer,
        }
    }

    pub fn step<'a>(&'a self, name: &str) -> Step<'a> {
        Step {
            exec: self,
            builder: WasiEnv::builder(name),
            binary: None,
            sysroot: None,
            union_fs: None,
            debug: false,
        }
    }

    pub async fn read_bytes(&self, path: &str) -> Result<Vec<u8>, std::io::Error> {
        let mut file = self.fs.new_open_options().read(true).open(path)?;
        let mut wasm_bytes = Vec::new();
        file.read_to_end(&mut wasm_bytes).await?;
        Ok(wasm_bytes)
    }

    #[allow(dead_code)]
    pub async fn write_bytes(&self, path: &str, bytes: &[u8]) -> Result<(), std::io::Error> {
        let mut file = self
            .fs
            .new_open_options()
            .write(true)
            .truncate(true)
            .open(path)?;
        file.write_all(&bytes).await?;
        Ok(())
    }
}

impl<'a> Step<'a> {
    /// Sets the binary to be executed for this step.
    ///
    /// If it starts with a "/", it is treated as an absolute path in the current filesystem.
    pub fn binary(mut self, url_or_path: &str) -> Self {
        self.binary = Some(String::from(url_or_path));
        self
    }

    /// Sets the sysroot to be used for this step.
    ///
    /// This should be a URL to a tarball which will be injected into the root of the filesystem.
    pub fn sysroot(mut self, url: &str) -> Self {
        self.sysroot = Some(String::from(url));
        self
    }

    /// Allows unioning a custom filesystem into this step's filesystem.
    ///
    /// The unioned filesystem will be layered on top of the sysroot, if any,
    /// potentially overwriting files in the sysroot if there are conflicts.
    pub fn fs(mut self, fs: Box<dyn FileSystem>) -> Self {
        self.union_fs = Some(fs);
        self
    }

    /// Adds arguments to be passed to argv.
    /// The program name is not included here.
    pub fn args<I, Arg>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = Arg>,
        Arg: AsRef<[u8]>,
    {
        self.builder.add_args(args);
        self
    }

    /// Enable/disable debug mode for this step
    pub fn debug(mut self, enable_debugging: bool) -> Self {
        self.debug = enable_debugging;
        self
    }

    pub async fn run(self) -> Result<(), RuntimeError> {
        /* Download the binary from the URL / filesystem */
        let Some(binary_loc) = &self.binary else {
            return Err(RuntimeError::new("No binary specified"));
        };

        /* In debug mode, we need to instrument the binary */
        let mut debugger = None;

        let binary_bytes = if binary_loc.starts_with("/") {
            let mut wasm = self
                .exec
                .read_bytes(binary_loc)
                .await
                .ensure("Read binary from filesystem")?;

            if self.debug {
                WorkerOut::Download {
                    data: wasm.clone(),
                    filename: "pre.wasm".into(),
                }
                .send();

                let InstrumenterResult { info, wasm } =
                    instrument_wasm(&wasm).ensure("Instrumented WASM")?;
                debugger = Some(WorkerDebugger::new(info));

                WorkerOut::Download {
                    data: wasm.clone(),
                    filename: "post.wasm".into(),
                }
                .send();
            }

            wasm
        } else {
            fetch_bytes(binary_loc)
                .await
                .ensure("Fetch binary from network")?
        };

        let mut store = Store::default();
        let module = Module::from_binary(&store, &binary_bytes).ensure("Created WASM module")?;

        /* Fetch the sysroot and union into the vfs if needed */
        if let Some(sysroot_loc) = &self.sysroot {
            let sysroot_bytes = fetch_bytes(sysroot_loc)
                .await
                .ensure("Fetched sysroot from network")?;
            let sysroot_fs = extract_tar_gz(sysroot_bytes)
                .await
                .ensure("Created sysroot fs from tarball")?;
            let sysroot_fs: Arc<dyn FileSystem + Send + Sync> = Arc::new(sysroot_fs);
            self.exec.fs.union(&sysroot_fs);
        }

        /* Union user files into the vfs if needed */
        if let Some(union_fs) = self.union_fs {
            let union_fs: Arc<dyn FileSystem + Send + Sync> = Arc::new(union_fs);
            self.exec.fs.union(&union_fs);
        }

        /* Configure Wasmer WASI environment */
        let mut builder = self
            .builder
            .runtime(JsRuntime::instance())
            .fs(Box::new(self.exec.fs.clone()))
            .stdout(Box::new(Stdout::new(StdoutMode::Out)))
            .stderr(Box::new(Stdout::new(StdoutMode::Err)))
            .stdin(Box::new(Stdin::new(&self.exec.stdin_buffer)));

        builder
            .add_preopen_dir("/")
            .ensure("Preopened root directory")?;

        /* Instantiate and run the binary */
        let instance = if let Some(debugger) = debugger {
            let wasi_env = builder.build().ensure("Built WASI environment")?;
            let mut wasi_func_env = WasiFunctionEnv::new(&mut store, wasi_env);

            let mut imports = wasi_func_env
                .import_object(&mut store, &module)
                .ensure("Created WASI import object")?;

            debugger.attach(&mut store, &mut imports);

            let instance = wasmer::Instance::new(&mut store, &module, &imports)
                .ensure("Created instance with debug imports")?;

            wasi_func_env
                .initialize(&mut store, instance.clone())
                .ensure("Initialized WASI")?;

            instance
        } else {
            let (instance, _env) = builder
                .instantiate(module, &mut store)
                .ensure("Instantiated Wasmer instance")?;
            instance
        };

        let start = instance
            .exports
            .get_function("_start")
            .ensure("Found _start export")?;

        /* Prevent `exit(0)` from being treated as an error */
        if let Err(err) = start.call(&mut store, &[]) {
            let wasi_err = err.downcast_ref::<WasiError>();
            let Some(wasi_err) = wasi_err else {
                return Err(err);
            };

            let WasiError::Exit(code) = wasi_err else {
                return Err(err);
            };

            if !code.is_success() {
                return Err(err);
            }
        }

        Ok(())
    }
}

pub async fn fetch_bytes(url: &str) -> Result<Vec<u8>, JsValue> {
    let scope = DedicatedWorkerGlobalScope::from(JsValue::from(js_sys::global()));
    let response: web_sys::Response = JsFuture::from(scope.fetch_with_str(url))
        .await?
        .dyn_into()?;
    let buffer = JsFuture::from(response.array_buffer()?).await?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

pub async fn extract_tar_gz(data: Vec<u8>) -> Result<mem_fs::FileSystem, std::io::Error> {
    let decoder = GzDecoder::new(Cursor::new(data));
    let mut archive = Archive::new(decoder);

    let fs = mem_fs::FileSystem::default();

    for entry in archive.entries()? {
        let mut entry = entry?;
        let is_dir = entry.header().entry_type().is_dir();
        let abs_path = PathBuf::from(format!("/{}", entry.path()?.to_string_lossy()));

        if is_dir {
            // in the case that a file is listed before the parent directory
            create_dir_all(&fs, &abs_path).expect("Created directory");
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

trait Ensure<T> {
    fn ensure(self, message: &str) -> Result<T, RuntimeError>;
}

impl<T, E: Debug> Ensure<T> for Result<T, E> {
    fn ensure(self, message: &str) -> Result<T, RuntimeError> {
        self.map_err(|e| RuntimeError::new(format!("{message}: {:?}", e)))
    }
}
