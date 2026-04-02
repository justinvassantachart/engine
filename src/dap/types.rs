use serde::Serialize;
use tsify::Tsify;

#[derive(Debug, Tsify, Serialize)]
pub struct StackFrame {
    pub id: u32,
    pub name: String,
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Tsify, Serialize)]
pub struct Variable {
    pub name: String,
    pub value: String,
    pub r#type: Option<String>,
}
