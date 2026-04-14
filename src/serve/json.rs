use crate::interpreter::Value;

pub fn json_to_value(json: &serde_json::Value) -> Value {
    crate::interpreter::value::json_to_value_ref(json).unwrap_or(Value::Null)
}

pub fn value_to_json(value: &Value) -> serde_json::Value {
    crate::interpreter::value::value_to_json(value).unwrap_or(serde_json::Value::Null)
}

pub fn convert_json_to_value(json: serde_json::Value) -> Value {
    crate::interpreter::value::json_to_value(json).unwrap_or(Value::Null)
}
