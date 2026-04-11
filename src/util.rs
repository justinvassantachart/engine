use wasm_bindgen::JsCast;
use web_sys::DedicatedWorkerGlobalScope;

/// Whether we are currently running inside a web worker
pub fn is_worker() -> bool {
    let global = js_sys::global();
    return global.is_instance_of::<DedicatedWorkerGlobalScope>();
}

/// Prints a formatted string to the JavaScript console.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {{
        let prefix = if $crate::util::is_worker() {
            "[runtime:worker] "
        } else {
            "[runtime:main] "
        };
        let body = format!("{}", format_args!($($arg)*));
        let fmt = {
            let mut s = String::with_capacity(prefix.len() + 6);
            s.push_str("%c");
            s.push_str(prefix);
            s.push_str("%c%s");
            s
        };
        web_sys::console::log_4(
            &wasm_bindgen::JsValue::from_str(&fmt),
            &wasm_bindgen::JsValue::from_str("font-weight: bold"),
            &wasm_bindgen::JsValue::from_str(""),
            &wasm_bindgen::JsValue::from_str(&body),
        );
    }};
}

/// Transforms `Result` into `Option` and logs an error if it occurs.
#[macro_export]
macro_rules! weak_error {
    ($res:expr) => {
        match $res {
            Ok(v) => Some(v),
            Err(e) => {
                $crate::log!("{:?}", e);
                None
            }
        }
    };
    ($res:expr, $msg:expr) => {
        match $res {
            Ok(v) => Some(v),
            Err(e) => {
                $crate::log!("{}: {:?}", $msg, e);
                None
            }
        }
    };
}

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
