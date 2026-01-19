use instant::Duration;
use std::future::ready;
use std::pin::Pin;
use std::sync::Arc;
use wasmer_wasix::runtime::Runtime;
use wasmer_wasix::runtime::resolver::{PackageSummary, QueryError, Source};
use wasmer_wasix::virtual_net;
use wasmer_wasix::{VirtualTaskManager, WasiThreadError, runtime::task_manager::TaskWasm};

#[derive(Debug)]
pub struct JsRuntime {
    task_manager: Arc<dyn VirtualTaskManager>,
    networking: Arc<dyn virtual_net::VirtualNetworking>,
}

impl JsRuntime {
    pub fn new() -> Self {
        Self {
            task_manager: Arc::new(UnsupportedTaskManager::default()),
            networking: Arc::new(virtual_net::UnsupportedVirtualNetworking::default()),
        }
    }

    pub fn instance() -> Arc<JsRuntime> {
        Arc::new(Self::new())
    }
}

impl Runtime for JsRuntime {
    fn networking(&self) -> &wasmer_wasix::virtual_net::DynVirtualNetworking {
        &self.networking
    }

    fn task_manager(&self) -> &std::sync::Arc<dyn wasmer_wasix::VirtualTaskManager> {
        &self.task_manager
    }

    fn source(&self) -> std::sync::Arc<dyn wasmer_wasix::runtime::resolver::Source + Send + Sync> {
        Arc::new(UnsupportedSource)
    }
}

/// From https://github.com/wasmerio/wasmer-js/blob/main/src/runtime.rs
/// A [`Source`] that will always error out with [`QueryError::Unsupported`].
#[derive(Debug, Clone)]
struct UnsupportedSource;

#[async_trait::async_trait]
impl Source for UnsupportedSource {
    async fn query(
        &self,
        package: &wasmer_config::package::PackageSource,
    ) -> Result<Vec<PackageSummary>, QueryError> {
        Err(QueryError::Unsupported {
            query: package.clone(),
        })
    }
}

// ============================================================================
// Task Manager
// ============================================================================

/// A handle to a threadpool backed by Web Workers.
#[derive(Default, Debug, Clone)]
pub struct UnsupportedTaskManager {}

// impl Drop for UnsupportedTaskManager {
//     fn drop(&mut self) {
//         tracing::debug!("Terminating ThreadPool");
//         // self.scheduler.close();
//     }
// }

#[async_trait::async_trait]
impl VirtualTaskManager for UnsupportedTaskManager {
    /// Invokes whenever a WASM thread goes idle. In some runtimes (like
    /// singlethreaded execution environments) they will need to do asynchronous
    /// work whenever the main thread goes idle and this is the place to hook
    /// for that.
    fn sleep_now(
        &self,
        _time: Duration,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + Sync + 'static>> {
        Box::pin(ready(()))
    }

    /// Starts an asynchronous task that will run on a shared worker pool
    /// This task must not block the execution or it could cause a deadlock
    fn task_shared(
        &self,
        _task: Box<
            dyn FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> + Send + 'static,
        >,
    ) -> Result<(), WasiThreadError> {
        Err(WasiThreadError::Unsupported)
    }

    /// Starts an asynchronous task will will run on a dedicated thread
    /// pulled from the worker pool that has a stateful thread local variable
    /// It is ok for this task to block execution and any async futures within its scope
    fn task_wasm(&self, _task: TaskWasm<'_>) -> Result<(), WasiThreadError> {
        Err(WasiThreadError::Unsupported)
    }

    /// Starts an asynchronous task will will run on a dedicated thread
    /// pulled from the worker pool. It is ok for this task to block execution
    /// and any async futures within its scope
    fn task_dedicated(
        &self,
        _task: Box<dyn FnOnce() + Send + 'static>,
    ) -> Result<(), WasiThreadError> {
        Err(WasiThreadError::Unsupported)
    }

    /// Returns the amount of parallelism that is possible on this platform
    fn thread_parallelism(&self) -> Result<usize, WasiThreadError> {
        Err(WasiThreadError::Unsupported)
    }

    fn spawn_with_module(
        &self,
        _module: wasmer::Module,
        _task: Box<dyn FnOnce(wasmer::Module) + Send + 'static>,
    ) -> Result<(), WasiThreadError> {
        Err(WasiThreadError::Unsupported)
    }
}
