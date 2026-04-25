# From `_handle_photo` to `uploader("photo", { ... })` — Soli's New Attachment Pipeline

Most file-upload code in MVC apps follows the same broken pattern: a 50-line handler in your controller that parses multipart, validates the file, encodes to base64, calls the storage backend, deletes the previous blob on replace, and threads error handling everywhere. Then your model needs a `*_url()` method, a cleanup callback, and a delete cascade. Then you need a controller for serving the bytes back. Then you need every project to copy this same boilerplate.

Soli now ships all of that out of the box.

## The model declares, the framework wires the rest

```soli
class Contact < Model
    uploader("photo", {
        "multiple":      false,
        "content_types": ["image/jpeg", "image/png", "image/webp"],
        "max_size":      2_000_000
    })
end
```

That's the entire upload story for a model. Here's what you get for free:

**Auto-generated instance methods** — every uploader synthesises `attach_<field>(file)`, `detach_<field>([blob_id])`, and `<field>_url(options?)` on the instance. You write `@contact.attach_photo(file)` and the framework's dispatcher routes through the model registry to the correct storage flow.

**Validation before storage** — content-type and size are checked *before* the bytes hit SoliDB. The previous CRM had a real bug here: it would store the blob, fetch its metadata back, then delete the blob if the metadata didn't match. Wrong content-type uploads wasted a round trip and risked orphans on delete-failure. Now: validate in-memory, reject early.

**Built-in `AttachmentsController`** — a generic show/create/destroy controller ships in the framework prelude, registered before user code loads. You don't add an `attachments_controller.sl` to your app. To override, define your own class with the same name — user controllers load after the prelude, so your version naturally shadows.

**Auto-mounted routes** — one line in `config/routes.sl`:

```soli
uploads("contacts", "photo")
```

…expands to:

```
GET    /contacts/:id/photo
POST   /contacts/:id/photo
DELETE /contacts/:id/photo
```

For `multiple: true` uploaders, you also get `:blob_id`-suffixed variants for showing/deleting individual blobs.

## A controller that's actually one line

The contacts edit form posts multipart to `/contacts/:id`. The controller's `update` action handles the photo without touching multipart parsing, base64, or SoliDB:

```soli
def update(req)
    @contact = Contact.find(params.id)
    if @contact.update(this._permit(params))
        return @contact.detach_photo() if params["remove_photo"] == "1"
        file = find_uploaded_file(req, "photo")
        @contact.attach_photo(file) unless file.nil?
        # ...
    end
end
```

The previous CRM's `_handle_photo` was 57 lines. The new version is functionally three. The deleted code was all incidental complexity — none of it described what the app actually does.

## Image transforms via URL

The same `GET /contacts/:id/photo` endpoint understands query parameters that drive Soli's [Image class](/docs/builtins/image). Want an avatar? Don't pre-generate three sizes at upload time — request the size you need from the URL:

```soli
<%= contact.photo_url({ "square": 100 }) %>
<!-- → <img src="/contacts/42/photo?v=abc&square=100"> -->
```

The full param list:

| Geometry  | Effects | Output |
|-----------|---------|--------|
| `w`, `h`  | `blur=2.5` | `fmt=webp` |
| `thumb=N` (max edge, aspect preserved) | `bright=15` (±) | `q=85` |
| `square=N` (exactly N×N, scale-and-crop) | `contrast=1.2` | |
| `crop=x,y,w,h` | `hue=90` | |
| `fit=cover` / `fit=contain` | `gray=1`, `invert=1` | |
| `flipx`, `flipy`, `rot=90/180/270` | | |

The pipeline order is fixed (crop → orient → resize → effects → encode), so identical params always produce identical bytes — and identical URLs. Browsers and CDNs cache them as separate entries; the cache buster `?v=<blob_id>` keeps the single-mode URL unique per blob, so replacing the photo invalidates every variant at once.

### Why three sizing modes?

The most common confusion is which size param to use. Given an 800×400 source:

- **`thumb=200`** or **`w=200` alone** → 200×100. Fits within 200×200, aspect preserved, smaller dim shrinks too.
- **`square=200`** → 200×200 *exactly*. Scale-and-crop, content cropped left/right, aspect preserved within the crop.
- **`w=200&h=200`** without `fit` → 200×200 *stretched*. Aspect distorted. Almost never what you want.

Reach for `square=N` for avatars and grid tiles — every cell is the same size, no CSS gymnastics. Use `thumb=N` when you need to bound the larger edge but keep the aspect.

## A 1000-pixel cap, server-side

Anyone who can hit `/contacts/:id/photo` can also append `?w=99999&h=99999`. Without bounds, that's a multi-gigabyte allocation per request — a denial-of-service waiting to happen. The framework clamps `w`, `h`, `thumb`, `square`, and the width/height components of `crop` to **1000 px**. Crafted URLs with bigger numbers are silently treated as 1000.

The cap is enforced in the framework prelude, not the URL builder, so no app can accidentally bypass it (and template typos can't either).

## What ships, what doesn't

**No server-side caching of transformed bytes.** Browser cache (`Cache-Control: public, max-age=86400`) and CDN handle the hot path. Cold cache misses re-run the transform pipeline — 50–200 ms for typical thumbnails on the Image class's pure-Rust pipeline. We considered persisting variants to disk under `<app_root>/.soli/cache/uploads/`, but the operational cost (eviction, multi-process coordination, disk monitoring) isn't worth it before there's evidence the transform pipeline is the bottleneck. If it ever is, that's a config flag away.

**No focal-point/smart crop** — `Image.crop` is rectangular. Face detection is a much bigger feature.

**No background/async storage hooks** — the helpers stay synchronous. SoliDB blob writes are fast enough that the typical request still completes in ms.

## The shape of the change

A summary, in numbers, from a single CRM dogfood:

- `crm/app/controllers/contacts_controller.sl`: **192 → 120 lines** (`_handle_photo` deleted, `_apply_photo` is 4 lines).
- `crm/app/controllers/attachments_controller.sl`: **deleted** (now framework-shipped).
- `crm/app/controllers/support.sl`: stripped to 2 functions (just `flash` and an optional `solidb_client` override).
- `crm/app/models/contact_model.sl`: dropped the manual `photo_url()` and the `before_delete` callback — both auto-generated.
- `crm/config/routes.sl`: dropped the inline `def uploads(...)` — now in the framework prelude.

Every new Soli app gets `uploader(...)`, `uploads(...)`, the AttachmentsController, and the URL-driven Image pipeline by default. No scaffolding, no copies, no drift. That's the framework doing what frameworks are supposed to do.

For the full param reference, see [Image transforms via URL](/docs/database/models#image-transforms-via-url) in the model docs. The override patterns and SoliDB env-var configuration live there too.
