//! Model template strings

/// Model template with placeholders
pub fn model_template(
    model_name: &str,
    collection_name: &str,
    field_comments: &str,
    validations: &str,
) -> String {
    format!(
        r#"# {model_name} model - auto-generated scaffold
# Collection: {collection_name}

class {model_name} < Model
  # Fields
  {field_comments}

  # Validations
  {validations}

  # Callbacks — name a method defined below, e.g.
  #   before_save("normalize")
  #   def normalize
  #     @title = @title.trim unless @title.blank?
  #   end
end
"#,
        model_name = model_name,
        collection_name = collection_name,
        field_comments = field_comments,
        validations = validations
    )
}
