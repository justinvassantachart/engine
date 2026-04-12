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

pub mod val_type_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use wasmparser::{RefType, ValType};

    pub fn serialize<S>(ty: &ValType, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let name = match ty {
            ValType::I32 => "i32",
            ValType::I64 => "i64",
            ValType::F32 => "f32",
            ValType::F64 => "f64",
            ValType::V128 => "v128",
            ValType::Ref(rt) => {
                if rt.is_func_ref() {
                    "funcref"
                } else if rt.is_extern_ref() {
                    "externref"
                } else {
                    "ref"
                }
            }
        };
        s.serialize_str(name)
    }

    pub fn deserialize<'de, D>(d: D) -> Result<ValType, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = <&str>::deserialize(d)?;
        Ok(match s {
            "i32" => ValType::I32,
            "i64" => ValType::I64,
            "f32" => ValType::F32,
            "f64" => ValType::F64,
            "v128" => ValType::V128,
            "funcref" => ValType::Ref(RefType::FUNCREF),
            "externref" => ValType::Ref(RefType::EXTERNREF),
            _ => ValType::Ref(RefType::FUNCREF),
        })
    }
}
