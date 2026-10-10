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
//! - `SOLI_ATTACHMENTS_MAX_DIMENSION` (default: `1000`) — ceiling for the
//!   `w`/`h`/`thumb`/`square`/`crop` transform parameters

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

# The services `store_attachment` and its siblings handle, as opposed to SoliDB
# blobs.
def __soli_bucket_service(service: Any) -> Bool
    service == "disk" || service == "s3" || service == "r2"
end

def attach_upload(model: Any, field_name: String, file: Any) -> Bool
    config = model_uploader_config(model.class, field_name)
    if config.nil?
        model._errors = [{ "message": "No uploader declared for #{field_name}." }]
        return false
    end

    # A finished resumable (tus) upload arrives as a reference, not bytes. Resolve
    # it before any check (it must belong to this session, be complete and
    # unexpired), so the limits below read the upload's own size and type, never
    # the ones the caller's hash claims. Disk and S3 attachments with no transform
    # stream it straight into storage; anything that needs the bytes (SoliDB
    # blobs, an image transform) loads them here, up to the inline limit.
    tus_id = file["tus_id"]
    service = config["service"] || "solidb"
    if !tus_id.nil?
        streams = __soli_bucket_service(service) && config["format"].nil? && config["max_width"].nil? && config["max_height"].nil?
        try
            file = tus_take(tus_id, !streams)
        catch error
            model._errors = [{ "message": "The upload for #{field_name} cannot be attached: #{error}" }]
            return false
        end
    end

    types = config["content_types"] ?? []
    if types.length() > 0 && !types.contains(file["content_type"])
        model._errors = [{ "message": "Unsupported file type for #{field_name}." }]
        return false
    end
    # SEC-031: `file["data"]` is now a base64-encoded string (was a
    # Vec<Int> array). Use the explicit `size` field for the size cap so
    # the comparison stays in raw-byte units, and pass `data` straight
    # to `solidb_store_blob` since it's already the base64 form the
    # blob store expects.
    if file["size"] > config["max_size"]
        max_mb = (config["max_size"] / 1000000).to_s
        model._errors = [{ "message": "#{field_name} must be under #{max_mb} MB." }]
        return false
    end

    # Storage-time image transform: convert the original to the configured
    # `format` (jpeg/png/webp) and/or downscale to `max_width`/`max_height`
    # before storing. No-op for non-image uploads or when no transform is
    # declared, so e.g. a PNG photo can be stored as a smaller lossy WebP.
    file = apply_uploader_transform(file, config)

    blob_id = null
    if __soli_bucket_service(service)
        blob_id = store_attachment(config, file)
    else
        client = __soli_resolve_solidb_client()
        blob_id = solidb_store_blob(client, config["collection"], file["data"],
                                    file["filename"], file["content_type"])
    end
    if blob_id.nil?
        model._errors = [{ "message": "Failed to store #{field_name}." }]
        return false
    end
    tus_discard(tus_id) if !tus_id.nil?

    __soli_link_blob(model, field_name, config, service, blob_id, file["content_type"])
end

# Record `blob_id` on the model (single or multiple) and delete the blob it
# replaces. Shared by `attach_upload` and `direct_upload_finish`.
#
# The content type goes next to the id — `<field>_content_type`, or a
# `{blob_id: type}` hash in `<field>_content_types` — so `upload_url` can end
# the URL in the original's extension without asking the store. A column-mode
# table without that column keeps the id alone, and its URLs no extension.
def __soli_link_blob(model: Any, field_name: String, config: Any, service: String, blob_id: Any, content_type: Any = null) -> Bool
    if config["multiple"]
        ids = model["#{field_name}_blob_ids"] ?? []
        ids.push(blob_id)
        changes = { "#{field_name}_blob_ids": ids }
        types_field = "#{field_name}_content_types"
        if !content_type.nil? && __soli_model_stores_field(model, types_field)
            changes[types_field] = (model[types_field] ?? {}).merge({ "#{blob_id}": content_type })
        end
        model.update(changes)
        return true
    end

    previous = model["#{field_name}_blob_id"]
    changes = { "#{field_name}_blob_id": blob_id }
    type_field = "#{field_name}_content_type"
    changes[type_field] = content_type if !content_type.nil? && __soli_model_stores_field(model, type_field)
    model.update(changes)
    if !previous.nil?
        if __soli_bucket_service(service)
            delete_attachment(config, previous)
        else
            client = __soli_resolve_solidb_client()
            solidb_delete_blob(client, config["collection"], previous)
        end
    end
    true
end

# Step 1 of a browser-to-bucket upload (s3 attachments only): validate what the
# browser says it will send, and return a presigned PUT for it:
#   { "id", "url", "method": "PUT", "headers": {...}, "expires_in" }
# The browser PUTs the file to `url` with those headers, then the app calls
# `direct_upload_finish`. The URL does not pin the size or type — S3 takes what
# it is sent — so the limits are enforced at finish, against what the bucket
# holds. On a refusal this returns `{ "error": "..." }`.
#
# The id is recorded in the session: finish accepts only an id this session
# started for this field, so a caller cannot claim (and, through the size check
# or a later replace, delete) a blob that is someone else's.
def direct_upload_start(model: Any, field_name: String, filename: String, content_type: String, size: Int, expires_in: Int = 900) -> Any
    config = model_uploader_config(model.class, field_name)
    return { "error": "No uploader declared for #{field_name}." } if config.nil?
    return { "error": "Direct uploads need an s3 attachment service." } if config["service"] != "s3"

    types = config["content_types"] ?? []
    return { "error": "Unsupported file type for #{field_name}." } if types.length() > 0 && !types.contains(content_type)
    return { "error": "#{field_name} must be under #{(config["max_size"] / 1000000).to_s} MB." } if size > config["max_size"]

    upload = direct_upload(config, { "filename": filename, "content_type": content_type, "size": size }, expires_in)
    started = session_get("__soli_direct_uploads") ?? []
    started.push("#{field_name}:#{upload["id"]}")
    # Bounded: the cookie session driver carries this in a ~4KB cookie.
    started = started.slice(started.length() - 10, started.length()) if started.length() > 10
    session_set("__soli_direct_uploads", started)
    upload
end

# Step 2: the browser says it is done. Ask the bucket what it holds under `id`
# (never trust the browser's word for size or type), re-check the limits against
# that, and attach it. `false` with `model._errors` when nothing arrived.
def direct_upload_finish(model: Any, field_name: String, blob_id: String) -> Bool
    config = model_uploader_config(model.class, field_name)
    if config.nil? || config["service"] != "s3"
        model._errors = [{ "message": "Direct uploads need an s3 attachment uploader for #{field_name}." }]
        return false
    end

    ticket = "#{field_name}:#{blob_id}"
    started = session_get("__soli_direct_uploads") ?? []
    if !started.contains(ticket)
        model._errors = [{ "message": "No upload for #{field_name} was started with that id." }]
        return false
    end

    head = direct_upload_head(config, blob_id)
    if head.nil?
        model._errors = [{ "message": "The upload for #{field_name} did not arrive." }]
        return false
    end
    # Arrived: the id is spent, whatever the checks below decide.
    session_set("__soli_direct_uploads", started.filter(fn(t) t != ticket))

    types = config["content_types"] ?? []
    if (types.length() > 0 && !types.contains(head["content_type"])) || head["size"] > config["max_size"]
        delete_attachment(config, blob_id)
        model._errors = [{ "message": "The uploaded file for #{field_name} is not allowed." }]
        return false
    end

    __soli_link_blob(model, field_name, config, "s3", blob_id, head["content_type"])
end

def detach_upload(model: Any, field_name: String, blob_id: Any = null) -> Bool
    config = model_uploader_config(model.class, field_name)
    return false if config.nil?

    service = config["service"] || "solidb"
    client = null
    client = __soli_resolve_solidb_client() if service == "solidb"
    if config["multiple"]
        return false if blob_id.nil?
        ids = model["#{field_name}_blob_ids"] ?? []
        return false if !ids.contains(blob_id)
        if __soli_bucket_service(service)
            delete_attachment(config, blob_id)
        else
            solidb_delete_blob(client, config["collection"], blob_id)
        end
        kept = ids.filter(fn(id) id != blob_id)
        changes = { "#{field_name}_blob_ids": kept }
        types = model["#{field_name}_content_types"]
        changes["#{field_name}_content_types"] = types.reject { |id, _| id == blob_id } unless types.nil?
        model.update(changes)
        return true
    end

    current = model["#{field_name}_blob_id"]
    return false if current.nil?
    if __soli_bucket_service(service)
        delete_attachment(config, current)
    else
        solidb_delete_blob(client, config["collection"], current)
    end
    changes = { "#{field_name}_blob_id": null }
    changes["#{field_name}_content_type"] = null unless model["#{field_name}_content_type"].nil?
    model.update(changes)
    true
end

# Read an attachment's bytes back, whichever service holds them.
#
# `attach_upload` already knows the disk/s3-vs-solidb switch and so does
# `AttachmentsController#show`, but neither is reachable from code that is not
# answering an HTTP request — and `<field>_url` is a URL, which is no use to a
# caller that wants the bytes. An EUI view is exactly that caller: it has to
# hand real bytes to `eui_asset` before a window can draw them. Without this
# every application re-implements the switch, and "choose your storage" stops
# being true the moment anything reads a file back.
#
# `owner` is the class, the record, or the class name. Answers `null` for a
# blob that is not there, which is the shape a caller already has to handle:
# a record can outlive its attachment.
def read_upload(owner: Any, field_name: String, blob_id: Any = null) -> Any
    return null if blob_id.nil?
    config = model_uploader_config(owner, field_name)
    return null if config.nil?

    service = config["service"] || "solidb"
    if __soli_bucket_service(service)
        return read_attachment(config, blob_id)
    end

    client = __soli_resolve_solidb_client()
    data = solidb_get_blob(client, config["collection"], blob_id)
    return null if data.nil?
    meta = solidb_get_blob_metadata(client, config["collection"], blob_id) ?? {}
    {
        "filename":     meta["filename"] ?? "file",
        "content_type": meta["content_type"] ?? "application/octet-stream",
        "size":         meta["size"] ?? 0,
        "data":         data
    }
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

        service = config["service"] || "solidb"
        query   = req["query"] ?? {}
        # On the edge build an R2 blob never crosses wasm: the Worker serves it
        # from the bucket, Range requests and image transforms included.
        return this._show_r2(req, config, blob_id, query, ctx["ext"]) if service == "r2" && __soli_edge()

        meta = null
        b64 = null
        if __soli_bucket_service(service)
            stored = read_attachment(config, blob_id)
            return halt(404, "Not found") if stored.nil?
            meta = stored
            b64 = stored["data"]
            query = this._apply_path_format(query, stored["content_type"] ?? "", ctx["ext"])
            return halt(404, "Not found") if query.nil?
        else
            client = __soli_resolve_solidb_client()
            meta   = solidb_get_blob_metadata(client, config["collection"], blob_id)
            ct     = meta["content_type"] ?? "application/octet-stream"
            query  = this._apply_path_format(query, ct, ctx["ext"])
            return halt(404, "Not found") if query.nil?

            # Unless an image transform needs the decoded pixels, the blob is
            # streamed from SoliDB to the client by the server, chunk by chunk,
            # with the client's `Range` forwarded — an audio player seeking in a
            # 150 MB episode gets the bytes it asked for, and no worker holds
            # the file. HEAD gets the headers alone.
            unless ct.starts_with("image/") && this._has_image_transforms(query)
                return solidb_blob_response(client, config["collection"], blob_id, req, {
                    "Content-Type":           ct,
                    "Content-Disposition":    this._disposition(ct),
                    "X-Content-Type-Options": "nosniff",
                    "Cache-Control":          "private, max-age=300"
                })
            end
            b64 = solidb_get_blob(client, config["collection"], blob_id)
        end

        # Apply image transforms if any are present in the query string. The
        # browser caches each (URL, query) combo separately, so subsequent
        # requests with the same params hit the browser cache and don't pay
        # for re-transformation. Disabled if the stored content-type isn't
        # an image — we just stream the raw bytes back.
        ct       = meta["content_type"] ?? "application/octet-stream"
        wants_xf = ct.starts_with("image/") && this._has_image_transforms(query)
        if wants_xf
            return this._render_transformed(b64, ct, query)
        end

        {
            "status":  200,
            "headers": {
                "Content-Type":           ct,
                "Content-Length":         str(meta["size"]),
                "Content-Disposition":    this._disposition(ct),
                "X-Content-Type-Options": "nosniff",
                "Cache-Control":          "private, max-age=300"
            },
            # `body_base64` hands the base64 straight to the response layer,
            # which decodes it once into bytes. `Base64.decode` here would
            # build a Soli array of one 16-byte Int per byte first -- ~16x the
            # attachment, transient, per concurrent download.
            "body_base64": b64
        }
    end

    # The response for an R2 blob on the edge build: the headers this controller
    # owns (type, disposition, nosniff, cache) and `x-soli-r2-object`, which
    # tells `worker.js` which object to answer with. The Worker streams it from
    # the bucket — honouring `Range` and `If-None-Match` — or, for an image
    # transform, through the Images binding (`[images] binding = "IMAGES"`),
    # falling back to the stored bytes when there is none.
    def _show_r2(req, config, blob_id, query, ext)
        head = __soli_r2_head(config, blob_id)
        return halt(404, "Not found") if head.nil?

        ct = head["content_type"] ?? "application/octet-stream"
        query = this._apply_path_format(query, ct, ext)
        return halt(404, "Not found") if query.nil?

        image = nil
        image = this._cf_image(query, ct) if ct.starts_with("image/") && this._has_image_transforms(query)
        {
            "status":  200,
            "headers": {
                "Content-Type":           ct,
                "Content-Disposition":    this._disposition(ct),
                "X-Content-Type-Options": "nosniff",
                "Cache-Control":          "private, max-age=300",
                "X-Soli-R2-Object":       __soli_r2_object(config, blob_id, image)
            },
            "body": ""
        }
    end

    # The transform query (`w`, `h`, `fit`, `crop`, `fmt`…) as options for the
    # Cloudflare Images binding: `{ "transform": {...}, "output": {...} }`.
    # Dimensions are clamped as for `_render_transformed`. Images has no hue
    # rotation or inversion, so `hue` and `invert` are ignored here.
    def _cf_image(query, original_ct)
        cap   = this._max_dimension()
        w     = this._int_param_clamped(query, "w", cap)
        h     = this._int_param_clamped(query, "h", cap)
        thumb = this._int_param_clamped(query, "thumb", cap)
        fit   = (query["fit"] ?? "").to_s
        square = this._int_param_clamped(query, "square", cap)
        if !square.nil?
            w   = square if w.nil?
            h   = square if h.nil?
            fit = "cover" if fit == ""
        end

        transform = {}
        crop = this._parse_crop(query)
        transform["trim"] = { "left": crop[0], "top": crop[1], "width": crop[2], "height": crop[3] } unless crop.nil?

        flip = ""
        flip = "h" if this._truthy(query, "flipx")
        flip = flip + "v" if this._truthy(query, "flipy")
        transform["flip"] = flip if flip != ""
        rot = this._int_param(query, "rot")
        transform["rotate"] = rot if [90, 180, 270].contains(rot)

        # The sizing precedence of `_render_transformed`: a thumbnail fits the
        # image in an N×N box, `w` alone too; `w` and `h` without a fit stretch.
        if !thumb.nil?
            transform["width"] = thumb
            transform["height"] = thumb
            transform["fit"] = "scale-down"
        elsif !w.nil? && !h.nil?
            transform["width"] = w
            transform["height"] = h
            transform["fit"] = ["cover", "contain"].contains(fit) ? fit : "squeeze"
        elsif !w.nil?
            transform["width"] = w
            transform["height"] = w
            transform["fit"] = "scale-down"
        end

        blur = this._float_param(query, "blur")
        transform["blur"] = this._clamp_f(blur * 2.0, 1.0, 250.0) unless blur.nil?
        bright = this._int_param(query, "bright")
        transform["brightness"] = this._clamp_f(1.0 + bright.to_f / 100.0, 0.0, 2.0) unless bright.nil?
        contrast = this._float_param(query, "contrast")
        transform["contrast"] = this._clamp_f(1.0 + contrast / 100.0, 0.0, 2.0) unless contrast.nil?
        transform["saturation"] = 0 if this._truthy(query, "gray")

        fmt = (query["fmt"] ?? "").to_s
        out_ct = original_ct
        out_ct = this._format_content_type(fmt) ?? original_ct if fmt != ""
        # What Images can write; anything else comes out as PNG.
        out_ct = "image/png" unless ["image/jpeg", "image/png", "image/gif", "image/webp", "image/avif"].contains(out_ct)
        output = { "format": out_ct }
        q = this._int_param(query, "q")
        output["quality"] = q unless q.nil?
        { "transform": transform, "output": output }
    end

    def _clamp_f(value, low, high)
        return low if value < low
        return high if value > high
        value
    end

    # This route serves attacker-supplied bytes from the application's own
    # origin, so two headers are not optional. `nosniff` (set next to this
    # wherever it is used) stops a browser re-typing a blob into something
    # executable, and anything the page does not mean to render inline is
    # forced to download.
    #
    # Inline: images, except `image/svg+xml`, which counts as executable — it
    # can carry script — not as an image. And audio/video, so a podcast plays
    # in the page's `<audio>` and in the browser's own player: a browser hands
    # those to its media pipeline, which decodes and never runs script, and
    # with `nosniff` an upload that lies about being `audio/mpeg` is refused
    # as media rather than rendered as HTML.
    def _disposition(ct)
        return "inline" if ct.starts_with("image/") && ct != "image/svg+xml"
        return "inline" if ct.starts_with("audio/") || ct.starts_with("video/")
        "attachment"
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
        field    = parts[2].split(".")[0]

        # `photo.webp`, `gallery/<blob_id>.webp`: only GET has routes that
        # take an extension, and the last segment carries it.
        ext = null
        last_pieces = parts[parts.length() - 1].split(".")
        ext = last_pieces[last_pieces.length() - 1] if last_pieces.length() > 1

        model_class = find_model_class_by_collection(resource)
        return null if model_class.nil?

        record = model_class.find(params.id)
        return null if record.nil?

        config = model_uploader_config(model_class, field)
        return null if config.nil?

        { "record": record, "field": field, "config": config, "ext": ext }
    end

    # `photo.webp` asks for what `photo?fmt=webp` asks for, under a name a CDN
    # caches by extension. The extension wins over a `fmt` in the query, since
    # it is what the response gets typed by; one that names the stored format
    # re-encodes nothing (`photo.jpg` of a JPEG is the raw bytes); one an image
    # cannot be encoded to is a 404 rather than a lie. Anything but an image
    # keeps its bytes whatever the extension: `cv.pdf` is only a name.
    def _apply_path_format(query, ct, ext)
        return query if ext.nil? || !ct.starts_with("image/")
        wanted = ext.downcase
        wanted = "jpg" if wanted == "jpeg"
        wanted = "tiff" if wanted == "tif"
        # The table `upload_url` builds the extension from, so every URL it
        # hands out for an original is the stored bytes here.
        return query.merge({ "fmt": "" }) if wanted == __soli_content_type_ext(ct)
        return null unless ["jpg", "png", "gif", "webp", "bmp", "tiff", "ico"].contains(wanted)
        query.merge({ "fmt": wanted })
    end

    # The Content-Type of an output format, `jpg` and `jpeg` alike: `fmt=jpg`
    # used to answer `image/jpg`, which is not a type.
    def _format_content_type(fmt)
        {
            "jpg": "image/jpeg", "jpeg": "image/jpeg", "png": "image/png",
            "gif": "image/gif", "webp": "image/webp", "bmp": "image/bmp",
            "tif": "image/tiff", "tiff": "image/tiff", "ico": "image/x-icon"
        }[fmt.downcase]
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
            # Dimensions are clamped so a crafted URL like `?w=99999&h=99999`
            # can't drive the worker into a multi-GB allocation. The cap
            # applies transitively: `_fit_image` resizes to a multiple of
            # (w, h) which are both capped here, and the crop parser caps its
            # own width/height components.
            #
            # 1000 px unless the app raises it — see `_max_dimension`.
            cap   = this._max_dimension()
            w     = this._int_param_clamped(query, "w", cap)
            h     = this._int_param_clamped(query, "h", cap)
            thumb = this._int_param_clamped(query, "thumb", cap)
            fit   = (query["fit"] ?? "").to_s

            # `square=N` is sugar for `w=N&h=N&fit=cover`. Only fills slots
            # the caller didn't set explicitly, so users can mix `square`
            # with their own overrides predictably.
            square = this._int_param_clamped(query, "square", cap)
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
                out_ct = this._format_content_type(fmt) ?? "image/" + fmt
            end

            q = this._int_param(query, "q")
            img = img.quality(q) unless q.nil?

            # Same reason as `show`: keep the encoded form and let the
            # response layer decode it once. Content-Length is left to the
            # server, which sets it from the decoded body.
            return {
                "status":  200,
                "headers": {
                    "Content-Type":           out_ct,
                    "X-Content-Type-Options": "nosniff",
                    "Cache-Control":          "public, max-age=86400"
                },
                "body_base64": img.to_buffer()
            }
        catch err
            # Transform failed — serve the original bytes so the page still
            # works. Short cache so a transient error doesn't poison the
            # browser cache for hours.
            return {
                "status":  200,
                "headers": {
                    "Content-Type":           original_ct,
                    "X-Content-Type-Options": "nosniff",
                    "Cache-Control":          "private, max-age=60"
                },
                "body_base64": b64
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
    # returns `[x, y, w, h]` with the width and height clamped to the same
    # ceiling as `w`/`h`/`thumb` (see `_max_dimension`) so a crafted
    # `crop=0,0,99999,99999` can't drive a giant allocation. x and y
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
        cap = this._max_dimension()
        for p in parts
            n = p.to_i
            return null if n == 0 && p != "0"
            return null if n < 0
            n = cap if idx >= 2 && n > cap
            coords.push(n)
            idx = idx + 1
        end
        coords
    end

    # Ceiling for every dimension parsed out of a transform query string:
    # `w`, `h`, `thumb`, `square`, and the width/height components of `crop`.
    #
    # 1000 px by default — comfortably above what a page actually displays,
    # and low enough that `?w=99999&h=99999` allocates nothing dangerous.
    # An app that genuinely serves larger images (retina heroes, a photo
    # gallery with a lightbox) raises it by setting
    # `SOLI_ATTACHMENTS_MAX_DIMENSION` in its `.env`.
    #
    # This is OPERATOR configuration, not request input, so the value is
    # trusted as given — but it is the ONLY thing standing between a crafted
    # URL and a `value × value` pixel allocation, so set it to what the app
    # really serves rather than to an arbitrarily large number.
    #
    # A missing, empty, non-numeric or non-positive value falls back to 1000:
    # a typo in `.env` must not silently remove the guard.
    def _max_dimension
        raw = getenv("SOLI_ATTACHMENTS_MAX_DIMENSION")
        return 1000 if raw.nil?
        n = raw.to_s.trim().to_i
        return 1000 if n < 1
        n
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The prelude is lexed and parsed at server startup, so a syntax slip in
    /// it doesn't break one route — it stops every app from booting. Parse
    /// both sources here so a typo surfaces in `cargo test` instead.
    #[test]
    fn prelude_sources_parse() {
        for (label, source) in [
            ("UPLOADS_PRELUDE_SOURCE", UPLOADS_PRELUDE_SOURCE),
            ("UPLOADS_HELPERS_SOURCE", UPLOADS_HELPERS_SOURCE),
        ] {
            let tokens = crate::lexer::Scanner::new(source)
                .scan_tokens()
                .unwrap_or_else(|e| panic!("{label} failed to lex: {e}"));
            crate::parser::Parser::new(tokens)
                .parse()
                .unwrap_or_else(|e| panic!("{label} failed to parse: {e}"));
        }
    }

    /// What `show` sends as `Content-Disposition`: media plays in place,
    /// images render, and everything else — SVG included — downloads.
    #[test]
    fn audio_and_video_are_inline_and_svg_still_downloads() {
        let mut interpreter = Interpreter::new();
        define_uploads_prelude(&mut interpreter).expect("prelude loads");
        interpret_source(
            &mut interpreter,
            r#"
__c = AttachmentsController.new()
__dispositions = [
    "audio/mpeg", "audio/mp4", "video/mp4", "image/png",
    "image/svg+xml", "text/html", "application/pdf"
].map(fn(ct) __c._disposition(ct)).join(",")
"#,
        )
        .expect("snippet runs");
        let got = interpreter.global_env().borrow().get("__dispositions");
        assert_eq!(
            got.map(|v| v.to_string()).as_deref(),
            Some("inline,inline,inline,inline,attachment,attachment,attachment")
        );
    }

    /// `photo.<ext>` becomes the `fmt` the query would have carried: the stored
    /// format re-encodes nothing, an image format is asked for, one an image
    /// cannot become is refused (nil → 404), and a non-image ignores it.
    #[test]
    fn a_path_extension_becomes_fmt() {
        let mut interpreter = Interpreter::new();
        define_uploads_prelude(&mut interpreter).expect("prelude loads");
        interpret_source(
            &mut interpreter,
            r#"
__c = AttachmentsController.new()
__show = fn(q) { q.nil? ? "404" : (q["fmt"] ?? "-") + "|" + (q["w"] ?? "-") }
__formats = [
    __show(__c._apply_path_format({"w": "9"}, "image/jpeg", nil)),
    __show(__c._apply_path_format({}, "image/jpeg", "jpg")),
    __show(__c._apply_path_format({"fmt": "png"}, "image/jpeg", "WEBP")),
    __show(__c._apply_path_format({}, "image/png", "pdf")),
    __show(__c._apply_path_format({}, "image/png", "svg")),
    __show(__c._apply_path_format({}, "image/svg+xml", "svg")),
    __show(__c._apply_path_format({}, "image/avif", "avif")),
    __show(__c._apply_path_format({}, "image/jpeg", "JPEG")),
    __show(__c._apply_path_format({}, "application/pdf", "pdf")),
    __c._format_content_type("jpg")
].join(",")
"#,
        )
        .expect("snippet runs");
        let got = interpreter.global_env().borrow().get("__formats");
        assert_eq!(
            got.map(|v| v.to_string()).as_deref(),
            Some("-|9,|-,webp|-,404,404,|-,|-,|-,-|-,image/jpeg")
        );
    }

    /// The transform query as Cloudflare Images options, for an R2 blob on
    /// the edge: the sizing precedence of `_render_transformed`, the same
    /// dimension cap, and an output format Images can write.
    #[test]
    fn a_transform_query_becomes_images_options() {
        let mut interpreter = Interpreter::new();
        define_uploads_prelude(&mut interpreter).expect("prelude loads");
        interpret_source(
            &mut interpreter,
            r#"
__c = AttachmentsController.new()
__images = [
    __c._cf_image({"w": "300", "fmt": "webp"}, "image/jpeg"),
    __c._cf_image({"square": "120", "gray": "1", "q": "70"}, "image/png"),
    __c._cf_image({"w": "99999", "h": "50", "crop": "1,2,30,40", "flipx": "1", "flipy": "1", "rot": "90"}, "image/jpeg"),
    __c._cf_image({"thumb": "64", "fmt": "bmp"}, "image/tiff")
].map(fn(o) o.to_json).join("\n")
"#,
        )
        .expect("snippet runs");
        let got = interpreter
            .global_env()
            .borrow()
            .get("__images")
            .map(|v| v.to_string())
            .unwrap_or_default();
        let options: Vec<serde_json::Value> = got
            .lines()
            .map(|line| serde_json::from_str(line).expect("json"))
            .collect();
        assert_eq!(
            options[0],
            serde_json::json!({"transform": {"width": 300, "height": 300, "fit": "scale-down"}, "output": {"format": "image/webp"}})
        );
        assert_eq!(
            options[1],
            serde_json::json!({"transform": {"width": 120, "height": 120, "fit": "cover", "saturation": 0}, "output": {"format": "image/png", "quality": 70}})
        );
        assert_eq!(
            options[2],
            serde_json::json!({"transform": {
                "trim": {"left": 1, "top": 2, "width": 30, "height": 40},
                "flip": "hv", "rotate": 90,
                "width": 1000, "height": 50, "fit": "squeeze"
            }, "output": {"format": "image/jpeg"}})
        );
        assert_eq!(
            options[3],
            serde_json::json!({"transform": {"width": 64, "height": 64, "fit": "scale-down"}, "output": {"format": "image/png"}})
        );
    }
}
