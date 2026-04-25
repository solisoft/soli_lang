//! Framework-shipped upload helpers + AttachmentsController.
//!
//! The Soli source below is interpreted at server startup so every app gets
//! the upload primitives "out of the box" without copying anything into
//! `app/controllers/`.
//!
//! The user can override any helper or the controller by defining a
//! same-named class or function in their `app/controllers/`. Soli's loader
//! processes user files after this prelude runs, so user definitions land
//! later and shadow the framework defaults.
//!
//! Configuration is read from environment variables with sensible defaults:
//! - `SOLIDB_HOST`     (default: `http://localhost:6745`)
//! - `SOLIDB_DATABASE` (default: `default`)
//! - `SOLIDB_USERNAME` (no default — auth skipped if not set)
//! - `SOLIDB_PASSWORD` (paired with username)

use crate::error::RuntimeError;
use crate::interpreter::executor::Interpreter;
use crate::span::Span;

/// Soli source for the upload helpers and the built-in `AttachmentsController`
/// class. Auto-loaded once per worker so user code (`uploader(...)` on
/// models, `uploads(...)` in routes, dot-syntax `@contact.attach_photo(file)`)
/// works without scaffolding files in every project.
pub(crate) const UPLOADS_PRELUDE_SOURCE: &str = r##"
    fn __soli_default_solidb_client() {
        let host = getenv("SOLIDB_HOST");
        if (host == null) { host = "http://localhost:6745"; }
        let database = getenv("SOLIDB_DATABASE");
        if (database == null) { database = "default"; }
        let client = Solidb(host, database);
        let user = getenv("SOLIDB_USERNAME");
        let pass = getenv("SOLIDB_PASSWORD");
        if (user != null) {
            if (pass == null) { pass = ""; }
            solidb_auth(client, user, pass);
        }
        return client;
    }

    fn __soli_resolve_solidb_client() {
        if (defined("solidb_client")) {
            return solidb_client();
        }
        return __soli_default_solidb_client();
    }
"##;

/// Soli source for the upload helpers (post-Solidb-client setup) and the
/// `AttachmentsController` class. Split from the `__soli_*` glue so the
/// `def`/`class` declarations parse with the indentation Soli expects.
pub(crate) const UPLOADS_HELPERS_SOURCE: &str = r##"
# `find_uploaded_file` and `upload_url` are registered as Rust natives in
# `interpreter::builtins::uploads::register_uploader_helpers` so they're
# also reachable from the template-render environment used by views.
# The remaining helpers below are pure Soli — controllers call them.

def attach_upload(model: Any, field_name: String, file: Any) -> Bool
    config = model_uploader_config(model.class, field_name)
    if config.nil?
        model._errors = [{ "message": "No uploader declared for #{field_name}." }]
        return false
    end

    if !config["content_types"].contains(file["content_type"])
        model._errors = [{ "message": "Unsupported file type for #{field_name}." }]
        return false
    end
    if file["data"].length() > config["max_size"]
        max_mb = (config["max_size"] / 1000000).to_s
        model._errors = [{ "message": "#{field_name} must be under #{max_mb} MB." }]
        return false
    end

    client = __soli_resolve_solidb_client()
    b64 = Base64.encode(file["data"])
    blob_id = solidb_store_blob(client, config["collection"], b64,
                                file["filename"], file["content_type"])
    if blob_id.nil?
        model._errors = [{ "message": "Failed to store #{field_name}." }]
        return false
    end

    if config["multiple"]
        ids = model["#{field_name}_blob_ids"] ?? []
        ids.push(blob_id)
        model.update({ "#{field_name}_blob_ids": ids })
        return true
    end

    previous = model["#{field_name}_blob_id"]
    model.update({ "#{field_name}_blob_id": blob_id })
    if !previous.nil?
        solidb_delete_blob(client, config["collection"], previous)
    end
    true
end

def detach_upload(model: Any, field_name: String, blob_id: Any = null) -> Bool
    config = model_uploader_config(model.class, field_name)
    return false if config.nil?

    client = __soli_resolve_solidb_client()
    if config["multiple"]
        return false if blob_id.nil?
        ids = model["#{field_name}_blob_ids"] ?? []
        return false if !ids.contains(blob_id)
        solidb_delete_blob(client, config["collection"], blob_id)
        kept = ids.filter(fn(id) id != blob_id)
        model.update({ "#{field_name}_blob_ids": kept })
        return true
    end

    current = model["#{field_name}_blob_id"]
    return false if current.nil?
    solidb_delete_blob(client, config["collection"], current)
    model.update({ "#{field_name}_blob_id": null })
    true
end

def detach_all_uploads(model: Any)
    fields = model_uploader_fields(model.class)
    for field in fields
        config = model_uploader_config(model.class, field)
        next if config.nil?
        if config["multiple"]
            for blob_id in (model["#{field}_blob_ids"] ?? [])
                detach_upload(model, field, blob_id)
            end
            next
        end
        detach_upload(model, field) unless model["#{field}_blob_id"].nil?
    end
end

class AttachmentsController < Controller
    def show(req)
        ctx = this._context(req)
        return halt(404, "Not found") if ctx.nil?
        record   = ctx["record"]
        field    = ctx["field"]
        config   = ctx["config"]
        blob_id  = this._target_blob_id(record, field, config, params["blob_id"])
        return halt(404, "Not found") if blob_id.nil?

        client = __soli_resolve_solidb_client()
        meta   = solidb_get_blob_metadata(client, config["collection"], blob_id)
        data   = Base64.decode(solidb_get_blob(client, config["collection"], blob_id))
        {
            "status":  200,
            "headers": {
                "Content-Type":   meta["content_type"] ?? "application/octet-stream",
                "Content-Length": str(meta["size"]),
                "Cache-Control":  "private, max-age=300"
            },
            "body": data
        }
    end

    def create(req)
        ctx = this._context(req)
        return halt(404, "Not found") if ctx.nil?
        file = find_uploaded_file(req, ctx["field"])
        return halt(400, "No file uploaded") if file.nil?

        record = ctx["record"]
        if attach_upload(record, ctx["field"], file)
            return { "status": 204, "body": "" }
        end
        msg = (record._errors[0] ?? { "message": "Upload failed" })["message"]
        { "status": 422, "body": msg }
    end

    def destroy(req)
        ctx = this._context(req)
        return halt(404, "Not found") if ctx.nil?
        ok = detach_upload(ctx["record"], ctx["field"], params["blob_id"])
        return halt(404, "Not found") unless ok
        { "status": 204, "body": "" }
    end

    def _context(req)
        parts = (req["path"] ?? "").split("/").filter(fn(s) s != "")
        return null if parts.length() < 3

        resource = parts[0]
        field    = parts[2]

        model_class = find_model_class_by_collection(resource)
        return null if model_class.nil?

        record = model_class.find(params.id)
        return null if record.nil?

        config = model_uploader_config(model_class, field)
        return null if config.nil?

        { "record": record, "field": field, "config": config }
    end

    def _target_blob_id(record, field, config, requested_id)
        if config["multiple"]
            return null if requested_id.nil?
            ids = record["#{field}_blob_ids"] ?? []
            return null unless ids.contains(requested_id)
            return requested_id
        end
        record["#{field}_blob_id"]
    end
end
"##;

/// Lex+parse+execute the embedded upload prelude into the given interpreter.
/// Called once per worker after model classes are registered and once on the
/// main thread before user routes are loaded. Idempotent — re-defining `def`s
/// and the `AttachmentsController` class is a no-op apart from the work.
pub(crate) fn define_uploads_prelude(interpreter: &mut Interpreter) -> Result<(), RuntimeError> {
    interpret_source(interpreter, UPLOADS_PRELUDE_SOURCE)?;
    interpret_source(interpreter, UPLOADS_HELPERS_SOURCE)
}

fn interpret_source(interpreter: &mut Interpreter, source: &str) -> Result<(), RuntimeError> {
    let tokens = crate::lexer::Scanner::new(source)
        .scan_tokens()
        .map_err(|e| RuntimeError::General {
            message: format!("Uploads prelude lexer error: {}", e),
            span: Span::default(),
        })?;
    let program =
        crate::parser::Parser::new(tokens)
            .parse()
            .map_err(|e| RuntimeError::General {
                message: format!("Uploads prelude parser error: {}", e),
                span: Span::default(),
            })?;
    interpreter.interpret(&program)
}
