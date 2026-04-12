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
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::util::__out!(::web_sys::console::log_3, $($arg)*)
    };
}
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
