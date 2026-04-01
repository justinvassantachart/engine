/// Prints a formatted string to the JavaScript console.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        web_sys::console::log_1(&format!($($arg)*).into());
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
