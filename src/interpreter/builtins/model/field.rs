//! Field definition system for Model/ORM.

#[derive(Debug, Clone, Default)]
pub struct FieldOptions {
    pub required: bool,
    pub unique: bool,
    pub index: bool,
    pub immutable: bool,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub pattern: Option<String>,
    pub embedded: bool,
    pub reference: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FieldDefinition {
    pub name: String,
    pub field_type: String,
    pub options: FieldOptions,
}

#[derive(Debug, Clone)]
pub enum FieldType {
    String,
    Int,
    Float,
    Bool,
    DateTime,
    Reference(String),
}

pub struct FieldBuilder {
    fields: Vec<FieldDefinition>,
}

impl FieldBuilder {
    pub fn new() -> Self {
        Self { fields: Vec::new() }
    }

    pub fn string(&mut self, name: &str) -> &mut FieldDefinition {
        self.fields.push(FieldDefinition {
            name: name.to_string(),
            field_type: "string".to_string(),
            options: FieldOptions::default(),
        });
        self.fields.last_mut().unwrap()
    }

    pub fn int(&mut self, name: &str) -> &mut FieldDefinition {
        self.fields.push(FieldDefinition {
            name: name.to_string(),
            field_type: "int".to_string(),
            options: FieldOptions::default(),
        });
        self.fields.last_mut().unwrap()
    }

    pub fn build(self) -> Vec<FieldDefinition> {
        self.fields
    }
}
