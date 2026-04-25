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
        b64    = solidb_get_blob(client, config["collection"], blob_id)

        # Apply image transforms if any are present in the query string. The
        # browser caches each (URL, query) combo separately, so subsequent
        # requests with the same params hit the browser cache and don't pay
        # for re-transformation. Disabled if the stored content-type isn't
        # an image — we just stream the raw bytes back.
        query    = req["query"] ?? {}
        ct       = meta["content_type"] ?? "application/octet-stream"
        wants_xf = ct.starts_with("image/") && this._has_image_transforms(query)
        if wants_xf
            return this._render_transformed(b64, ct, query)
        end

        {
            "status":  200,
            "headers": {
                "Content-Type":   ct,
                "Content-Length": str(meta["size"]),
                "Cache-Control":  "private, max-age=300"
            },
            "body": Base64.decode(b64)
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

    # Truthy if any of the recognised image-transform query keys are set.
    # The list mirrors what `upload_url(...)` accepts in its options hash.
    def _has_image_transforms(query)
        return false if query.nil?
        for key in [
            "w", "h", "thumb", "square", "crop", "fit",
            "flipx", "flipy", "rot",
            "blur", "bright", "contrast", "hue",
            "gray", "invert",
            "fmt", "q"
        ]
            v = query[key]
            return true if !v.nil? && v != ""
        end
        false
    end

    # Decode the stored blob, run the requested transforms via Image, and
    # return a response hash. Falls back to raw bytes on any error so a
    # broken transform never blocks the unmodified asset from rendering.
    #
    # IMPORTANT: each branch returns explicitly. Soli's `try` statement drops
    # the `Normal(value)` payload from its body, so a trailing-expression
    # value would surface as `null` to the caller. Explicit `return` routes
    # through `ControlFlow::Return(v)` which the try-handler propagates.
    #
    # Pipeline order:
    #   1. crop     — pick a source region first so everything else operates
    #                 on the user-selected pixels.
    #   2. flip/rot — orientation. Done before sizing so subsequent w/h
    #                 apply to the rotated frame.
    #   3. resize   — thumb / fit-cover / fit-contain / exact / square-thumb.
    #   4. effects  — blur, bright, contrast, hue, invert, gray.
    #   5. encode   — fmt + quality.
    def _render_transformed(b64, original_ct, query)
        try
            img = Image.from_buffer(b64)

            # 1. Crop source region.
            crop = this._parse_crop(query)
            img = img.crop(crop[0], crop[1], crop[2], crop[3]) unless crop.nil?

            # 2. Orientation.
            img = img.flip_horizontal() if this._truthy(query, "flipx")
            img = img.flip_vertical()   if this._truthy(query, "flipy")
            rot = this._int_param(query, "rot")
            if rot == 90
                img = img.rotate90()
            elsif rot == 180
                img = img.rotate180()
            elsif rot == 270
                img = img.rotate270()
            end

            # 3. Sizing. Order of precedence:
            #    - `thumb` (square fit, max edge)
            #    - `square=N` shorthand → fills w=N, h=N, fit=cover unless
            #      they're already set explicitly
            #    - `fit=cover` + w/h  (fill, crop overflow)
            #    - `fit=contain` + w/h (fit inside, preserve aspect)
            #    - `w + h` (exact resize, may distort)
            #    - `w` alone (square thumbnail at that max edge)
            #
            # Dimensions are clamped to 1000 px so a crafted URL like
            # `?w=99999&h=99999` can't drive the worker into a multi-GB
            # allocation. The cap applies transitively: `_fit_image` resizes
            # to a multiple of (w, h) which are both capped here, and the
            # crop parser caps its own width/height components.
            w     = this._int_param_clamped(query, "w", 1000)
            h     = this._int_param_clamped(query, "h", 1000)
            thumb = this._int_param_clamped(query, "thumb", 1000)
            fit   = (query["fit"] ?? "").to_s

            # `square=N` is sugar for `w=N&h=N&fit=cover`. Only fills slots
            # the caller didn't set explicitly, so users can mix `square`
            # with their own overrides predictably.
            square = this._int_param_clamped(query, "square", 1000)
            if !square.nil?
                w   = square if w.nil?
                h   = square if h.nil?
                fit = "cover" if fit == ""
            end
            # Pulled out as a let because Soli's parser stumbles on
            # `elsif (a || b) && c` — see _has_image_transforms's helper notes.
            fit_mode    = fit == "cover" || fit == "contain"
            has_w_and_h = !w.nil? && !h.nil?
            if !thumb.nil?
                img = img.thumbnail(thumb)
            elsif fit_mode && has_w_and_h
                img = this._fit_image(img, fit, w, h)
            elsif has_w_and_h
                img = img.resize(w, h)
            elsif !w.nil?
                img = img.thumbnail(w)
            end

            # 4. Effects.
            blur_val = this._float_param(query, "blur")
            img = img.blur(blur_val) unless blur_val.nil?

            bright = this._int_param(query, "bright")
            img = img.brightness(bright) unless bright.nil?

            contrast_val = this._float_param(query, "contrast")
            img = img.contrast(contrast_val) unless contrast_val.nil?

            hue_val = this._int_param(query, "hue")
            img = img.hue_rotate(hue_val) unless hue_val.nil?

            img = img.invert()    if this._truthy(query, "invert")
            img = img.grayscale() if this._truthy(query, "gray")

            # 5. Encode.
            fmt = query["fmt"]
            out_ct = original_ct
            if !fmt.nil? && fmt != ""
                img = img.format(fmt)
                out_ct = "image/" + fmt
            end

            q = this._int_param(query, "q")
            img = img.quality(q) unless q.nil?

            data = Base64.decode(img.to_buffer())
            return {
                "status":  200,
                "headers": {
                    "Content-Type":   out_ct,
                    "Content-Length": str(data.length()),
                    "Cache-Control":  "public, max-age=86400"
                },
                "body": data
            }
        catch err
            # Transform failed — serve the original bytes so the page still
            # works. Short cache so a transient error doesn't poison the
            # browser cache for hours.
            return {
                "status":  200,
                "headers": {
                    "Content-Type":   original_ct,
                    "Cache-Control":  "private, max-age=60"
                },
                "body": Base64.decode(b64)
            }
        end
    end

    # `fit=cover`: scale until the box is fully covered, then center-crop the
    # overflow → output is exactly w × h.
    # `fit=contain`: scale until the image fits within w × h with preserved
    # aspect → output is at most w × h.
    def _fit_image(img, mode, w, h)
        src_w = img.width
        src_h = img.height
        return img if src_w == 0 || src_h == 0
        ratio_w = w.to_f / src_w.to_f
        ratio_h = h.to_f / src_h.to_f
        scale = (mode == "cover") ? this._max_f(ratio_w, ratio_h) : this._min_f(ratio_w, ratio_h)
        scaled_w = (src_w.to_f * scale).to_i
        scaled_h = (src_h.to_f * scale).to_i
        scaled_w = 1 if scaled_w < 1
        scaled_h = 1 if scaled_h < 1
        resized = img.resize(scaled_w, scaled_h)
        return resized if mode != "cover"
        # Center-crop to (w, h)
        x = (scaled_w - w) / 2
        y = (scaled_h - h) / 2
        x = 0 if x < 0
        y = 0 if y < 0
        crop_w = (w < scaled_w) ? w : scaled_w
        crop_h = (h < scaled_h) ? h : scaled_h
        resized.crop(x, y, crop_w, crop_h)
    end

    def _max_f(a, b)
        a > b ? a : b
    end

    def _min_f(a, b)
        a < b ? a : b
    end

    # Parse `crop=x,y,w,h`. Returns `null` on any malformed component, otherwise
    # returns `[x, y, w, h]` with the width and height clamped to 1000 px so a
    # crafted `crop=0,0,99999,99999` can't drive a giant allocation. x and y
    # are not clamped — they're offsets into the source image, and Image.crop
    # rejects out-of-bounds offsets (the `try/catch` in _render_transformed
    # then falls back to streaming the raw blob).
    def _parse_crop(query)
        v = (query["crop"] ?? "").to_s
        return null if v == ""
        parts = v.split(",")
        return null if parts.length() != 4
        coords = []
        idx = 0
        for p in parts
            n = p.to_i
            return null if n == 0 && p != "0"
            return null if n < 0
            n = 1000 if idx >= 2 && n > 1000
            coords.push(n)
            idx = idx + 1
        end
        coords
    end

    # Truthy if `query[key]` is set, non-empty, and not "0". Used for boolean
    # flag params (gray, invert, flipx, flipy).
    def _truthy(query, key)
        v = query[key]
        return false if v.nil?
        s = v.to_s
        s != "" && s != "0"
    end

    # Same shape as `_int_param` but for floating-point values (blur, contrast).
    # Accepts an Int verbatim, a Float, or a numeric string like "3.5". Returns
    # null on anything else so the caller can skip the call.
    def _float_param(query, key)
        v = query[key]
        return null if v.nil? || v == ""
        return v.to_f if v.is_a?("Int") || v.is_a?("Float")
        s = v.to_s
        f = s.to_f
        return null if f == 0.0 && s != "0" && s != "0.0" && s != "0.00"
        f
    end

    # `_int_param` with a max-value clamp. Used for dimension params (`w`,
    # `h`, `thumb`) so a crafted `?w=99999` can't drive the worker into a
    # huge allocation. Returns null on malformed input, the parsed value
    # capped at `max` otherwise.
    def _int_param_clamped(query, key, max)
        n = this._int_param(query, key)
        return null if n.nil?
        return max if n > max
        n
    end

    def _int_param(query, key)
        v = query[key]
        return null if v.nil? || v == ""
        return v if v.is_a?("Int")
        n = v.to_i
        return null if n == 0 && v != "0"
        n
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
