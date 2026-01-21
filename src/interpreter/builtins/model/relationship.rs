//! Relationship manager for Model associations.

#[derive(Debug, Clone)]
pub enum RelationshipType {
    HasMany,
    HasOne,
    BelongsTo,
    Embedded,
}

#[derive(Debug, Clone)]
pub struct Relationship {
    pub name: String,
    pub relationship_type: RelationshipType,
    pub model: String,
    pub via: Option<String>,
}

impl Relationship {
    pub fn new(name: &str, relationship_type: RelationshipType, model: &str) -> Self {
        Self {
            name: name.to_string(),
            relationship_type,
            model: model.to_string(),
            via: None,
        }
    }

    pub fn via(mut self, field: &str) -> Self {
        self.via = Some(field.to_string());
        self
    }
}

#[derive(Debug, Default)]
pub struct RelationshipStore {
    relationships: Vec<Relationship>,
}

impl RelationshipStore {
    pub fn new() -> Self {
        Self {
            relationships: Vec::new(),
        }
    }

    pub fn has_many(&mut self, name: &str, model: &str) {
        self.relationships
            .push(Relationship::new(name, RelationshipType::HasMany, model));
    }

    pub fn has_one(&mut self, name: &str, model: &str) {
        self.relationships
            .push(Relationship::new(name, RelationshipType::HasOne, model));
    }

    pub fn belongs_to(&mut self, name: &str, model: &str) {
        self.relationships
            .push(Relationship::new(name, RelationshipType::BelongsTo, model));
    }

    pub fn embedded(&mut self, name: &str, model: &str) {
        self.relationships
            .push(Relationship::new(name, RelationshipType::Embedded, model));
    }
}
