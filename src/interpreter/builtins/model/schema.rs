//! JSON Schema generation for Model validation.

pub fn generate_json_schema(fields: &[String], validation_mode: &str) -> serde_json::Value {
    let mut properties = serde_json::Map::new();

    for field in fields {
        properties.insert(
            field.clone(),
            serde_json::json!({
                "type": "string"
            }),
        );
    }

    serde_json::json!({
        "type": "object",
        "properties": properties,
        "validation_mode": validation_mode
    })
}
