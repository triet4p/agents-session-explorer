//! JSON rendering shared by CLI command modules.

use serde_json::Value;


pub(crate) fn emit(v: &Value, _args: &[String]) {
    println!("{}", serde_json::to_string_pretty(v).unwrap());
}
