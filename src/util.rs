use std::ops::Deref;
use std::rc::{Rc, Weak};

#[doc(hidden)]
macro_rules! __out {
    ($log_fn:path, $($arg:tt)*) => {{
        $log_fn(
            &::wasm_bindgen::JsValue::from_str(&format!(
                "%c[{}] %s",
                concat!(file!(), ":\u{200B}", line!()),
            )),
            &::wasm_bindgen::JsValue::from_str("font-weight:bold"),
            &::wasm_bindgen::JsValue::from_str(&format!($($arg)*)),
        );
    }};
}

pub(crate) use __out;

/// Prints a formatted string to the JavaScript console.
#[allow(unused)]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::util::__out!(::web_sys::console::log_3, $($arg)*)
    };
}
#[allow(unused_imports)]
pub(crate) use log;

/// Prints a formatted warning to the JavaScript console.
macro_rules! warning {
    ($($arg:tt)*) => {
        $crate::util::__out!(::web_sys::console::warn_3, $($arg)*)
    };
}
pub(crate) use warning;

/// Transforms `Result` into `Option` and logs a warning if an error occurs.
macro_rules! weak_error {
    ($res:expr) => {
        match $res {
            Ok(v) => Some(v),
            Err(e) => {
                $crate::util::warning!("{:?}", e);
                None
            }
        }
    };
    ($res:expr, $msg:expr) => {
        match $res {
            Ok(v) => Some(v),
            Err(e) => {
                $crate::util::warning!("{}: {:?}", $msg, e);
                None
            }
        }
    };
}
pub(crate) use weak_error;

/// Checks if [WASM multi-memory](https://developer.mozilla.org/en-US/docs/WebAssembly/Reference/JavaScript_interface/Memory#webassembly.multiMemory)
/// is supported on this platform.
///
/// Some platforms (like Bun) do not support this feature since it is relatively new.
/// If it is not supported, we will place the debug stack in the same memory as program memory.
pub(crate) fn supports_wasm_multi_memory() -> bool {
    // Minimal module with two memory imports.
    const MULTI_MEMORY_PROBE: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, // \0asm
        0x01, 0x00, 0x00, 0x00, // version
        0x02, 0x20, // import section, 32 bytes
        0x02, // 2 imports
        0x05, b'd', b'e', b'b', b'u', b'g', // module "debug"
        0x06, b'm', b'e', b'm', b'o', b'r', b'y', // name "memory"
        0x02, 0x00, 0x01, // memory min=1
        0x05, b'd', b'e', b'b', b'u', b'g', // module "debug"
        0x05, b's', b't', b'a', b'c', b'k', // name "stack"
        0x02, 0x00, 0x01, // memory min=1
    ];
    js_sys::WebAssembly::validate(&js_sys::Uint8Array::from(MULTI_MEMORY_PROBE)).unwrap_or(false)
}

// ╭──────────────────────────────────────────────────────────────────────────╮
// │ Weak References                                                          │
// ╰──────────────────────────────────────────────────────────────────────────╯

/// Represents a shared reference to a long-lived value.
///
/// Normal Rust shared references (Rc<T>) will not deallocate T while there
/// are still references to it, whether weak or strong references. This is
/// problematic for our codebase, as we will pass around objects which hold weak
/// references to the JavaScript side, creating a potential for large memory leaks.
///
/// Paired with [WeakRef], we circumvent this problem by holding a shared reference
/// to a Box<T> which will properly deallocate its contents when the shared reference
/// count reaches 0. Think of this as a lightweight alternative to `Rc<T>`.
#[derive(Clone)]
pub struct Ref<T>(Rc<Box<T>>);

/// Represents a weak reference to a long-lived shared value.
pub struct WeakRef<T>(Weak<Box<T>>);

impl<T> Ref<T> {
    pub fn new_cyclic(data: impl FnOnce(&WeakRef<T>) -> T) -> Self {
        Ref(Rc::new_cyclic(|weak| {
            Box::new(data(&WeakRef(weak.clone())))
        }))
    }
}

impl<T> Deref for Ref<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> Clone for WeakRef<T> {
    fn clone(&self) -> Self {
        WeakRef(self.0.clone())
    }
}

impl<T> WeakRef<T> {
    pub fn as_deref(&self) -> Option<&T> {
        let ptr = self.0.as_ptr();
        if ptr.is_null() {
            return None;
        }
        Some(unsafe { &**ptr })
    }
}
