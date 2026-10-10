# Blog Controller
# Handles displaying blog posts (markdown content)

def index
    let posts = get_blog_posts()
    
    render("blog/index", {
        "title": "Blog",
        "layout": "layouts/docs",
        "breadcrumb_href": "/docs/blog",
        "posts": posts
    })
end

def tag_chip_class(tag)
    return "bg-sky-500/15 text-sky-300 border-sky-500/20"             if tag == "Tutorial"
    return "bg-purple-500/15 text-purple-300 border-purple-500/20"    if tag == "Architecture"
    return "bg-rose-500/15 text-rose-300 border-rose-500/20"          if tag == "Security"
    return "bg-amber-500/15 text-amber-300 border-amber-500/20"       if tag == "Philosophy"
    return "bg-emerald-500/15 text-emerald-300 border-emerald-500/20" if tag == "Feature"
    return "bg-fuchsia-500/15 text-fuchsia-300 border-fuchsia-500/20" if tag == "Deep Dive"
    return "bg-cyan-500/15 text-cyan-300 border-cyan-500/20"          if tag == "Guide"
    "bg-white/5 text-gray-400 border-white/10"
end

def tag_gradient_class(tag)
    return "from-sky-500/30 via-sky-600/10 to-slate-950"        if tag == "Tutorial"
    return "from-purple-500/30 via-purple-600/10 to-slate-950"  if tag == "Architecture"
    return "from-rose-500/30 via-rose-600/10 to-slate-950"      if tag == "Security"
    return "from-amber-500/30 via-amber-600/10 to-slate-950"    if tag == "Philosophy"
    return "from-emerald-500/30 via-emerald-600/10 to-slate-950" if tag == "Feature"
    return "from-fuchsia-500/30 via-fuchsia-600/10 to-slate-950" if tag == "Deep Dive"
    return "from-cyan-500/30 via-cyan-600/10 to-slate-950"      if tag == "Guide"
    "from-indigo-500/25 via-indigo-600/10 to-slate-950"
end

# Simple list - ordered manually (newest first)
def blog_manifest()
    [
        {"slug": "postgres-images-and-queues-on-the-edge", "file": "docs/blog/postgres-images-and-queues-on-the-edge.md", "desc": "Soli 2.21 brings what a real app reaches for to a Cloudflare Worker: uploads, images and video kept in R2 and served with Range and Cloudflare Images transforms; your Postgres through Hyperdrive, transactions included; background jobs on Cloudflare Queues. And a smaller Worker — 2.94 MB, under the free plan's 3 MB — because the image codecs left it.", "tag": "Feature", "image": "postgres-images-and-queues-on-the-edge.svg", "publish_on": "2026-10-10"},
        {"slug": "soli-on-cloudflare-workers-and-d1", "file": "docs/blog/soli-on-cloudflare-workers-and-d1.md", "desc": "A Soli app now runs inside a Cloudflare Worker — the same routes, controllers, views and models soli serve runs, answered in the datacenter nearest the visitor — and its models can keep their data in D1, Cloudflare's SQLite. Four commands to ship one, a notes app on D1 to copy, how a synchronous interpreter waits on an asynchronous database (JavaScript Promise Integration), and what stays with soli serve.", "tag": "Feature", "image": "soli-on-cloudflare-workers-and-d1.svg", "publish_on": "2026-10-10"},
        {"slug": "the-interpreter-in-every-closure", "file": "docs/blog/the-interpreter-in-every-closure.md", "desc": "Validators, scopes, mailer actions, perform_now and methods added to String each cost about 270 µs a call: every call built a whole interpreter and registered every builtin again, then ran the body in the environment its closure already had. Why, the two constructs that paid it twice, the fix, and 50 to 150 times less CPU. Plus a leak found on the way: in production, render_json skipped as_json and sent every field.", "tag": "Deep Dive", "image": "the-interpreter-in-every-closure.svg", "publish_on": "2026-10-08"},
        {"slug": "pocket-garden-eui", "file": "docs/blog/pocket-garden-eui.md", "desc": "Build a little gardening game in a native window, from one script: plant, water, harvest and press your crops into a herbarium. Six files joined with import and export, the rules in a class, plants drawn with canvas paths in theme colours, a wake clock that ticks once a second only while something grows, three screens in one window, and a copy you can play in the page.", "tag": "Tutorial", "image": "pocket-garden.svg", "publish_on": "2026-10-01"},
        {"slug": "deploy-with-soli-proxy", "file": "docs/blog/deploy-with-soli-proxy.md", "desc": "From a bare Linux server to push-to-deploy: soli-proxy under systemd with Let's Encrypt certificates it requests by itself, an app folder named after its domain, and a GitHub Actions workflow that boots the app, rsyncs it, runs migrations and calls soli-proxy deploy, which moves traffic only once the new slot answers /up. The settings that are required, and what to check when a step fails.", "tag": "Guide", "image": "deploy-with-soli-proxy.svg", "publish_on": "2026-10-06"},
        {"slug": "testing-gaps", "file": "docs/blog/testing-gaps.md", "desc": "The comparison page listed what Soli lacked, so we checked each entry against the code and closed the real ones: test filter, fail-fast, watch and Mock stubs and spies; migration rollbacks by step or version; resumable (tus), direct-to-S3 and pausable uploads; live_update across processes; OIDC introspection, registration, device grant and request objects; four more OAuth providers; component classes. What each cost, and the bugs the end-to-end runs caught.", "tag": "Feature", "publish_on": "2026-09-30"},
        {"slug": "why-actions-skipped-the-vm", "file": "docs/blog/why-actions-skipped-the-vm.md", "desc": "The VM got 2.6's speedups but req/s barely moved: a bare `name = value` compiled to an undefined-global write, so nearly every action fell back to the interpreter, and one that had already written answered 500. Fixing it meant closing every gap a write exposed: model members, query builders, a trailing if. Every action now runs on the VM: 40.7k to 106.3k req/s.", "tag": "Deep Dive", "image": "why-actions-skipped-the-vm.svg", "publish_on": "2026-10-05"},
        {"slug": "native-solidb-driver", "file": "docs/blog/native-solidb-driver.md", "desc": "Plain SoliDB reads on the native driver now decode each row once, straight from MessagePack into Soli values, instead of going through a serde_json tree first. Which reads qualify and which don't, why the driver used to be missing from release builds, how to turn it on with SOLI_DB_DRIVER=1, and how CI tests it against a real SoliDB.", "tag": "Deep Dive", "image": "native-solidb-driver.svg", "publish_on": "2026-10-04"},
        {"slug": "closure-cycles", "file": "docs/blog/closure-cycles.md", "desc": "A closure stored on `this` captures the environment that holds `this`, so instance and closure keep each other alive and reference counting never frees either: 362 MiB for 200,000 instances against 34 MiB with a method name. The new smell/closure-cycle lint rule, its fixes and blind spots, and the same cycle inside the runtime.", "tag": "Deep Dive", "image": "closure-cycles.svg", "publish_on": "2026-10-03"},
        {"slug": "imap-xoauth2", "file": "docs/blog/imap-xoauth2.md", "desc": "A support-inbox job that works when app passwords are switched off: Imap.new with an XOAUTH2 token, headers-only listing so bodies and attachments stay on the server until needed, the new uid_* verbs because a sequence number shifts as soon as a message moves, and replies that thread through the mailer's headers and alternatives.", "tag": "Guide", "image": "imap-xoauth2.svg", "publish_on": "2026-10-02"},
        {"slug": "splitting-the-request-path", "file": "docs/blog/splitting-the-request-path.md", "desc": "The largest god function, register_model_class at 4,329 lines, was split first as a pure move, and the four in the HTTP server followed: the listener, the dispatch cascade, the worker pipeline and the 276-line closure that finished every request. Nothing a Soli program can see changed, except one bug the split surfaced.", "tag": "Architecture", "image": "splitting-the-request-path.svg", "publish_on": "2026-10-01"},
        {"slug": "pdf-previews", "file": "docs/blog/pdf-previews.md", "desc": "Turn the same PDF template into page images for thumbnails, review screens and emails. pdf_preview paints the layout engine's own laid-out page, so the image cannot drift from the PDF: dpi, width/height and page selection, WebP at 51 KB against 165 KB of PNG, and an invoice controller with a thumbnail cache.", "tag": "Feature", "image": "pdf-previews.svg", "publish_on": "2026-09-30"},
        {"slug": "soli-check-false-positives", "file": "docs/blog/soli-check-false-positives.md", "desc": "On a 247-file application soli check reported 237 errors, all false, which is how a checker gets switched off. How the known-globals list stopped being hand-kept and is now read from the runtime, why a directory is checked as one namespace the way the server loads it, and why a lint rule must propose a pure rename or say it isn't one.", "tag": "Philosophy", "image": "soli-check-false-positives.svg", "publish_on": "2026-09-29"},
        {"slug": "eui-islands-and-resumable-sessions", "file": "docs/blog/eui-islands-and-resumable-sessions.md", "desc": "A socket costs a session per reader, even one who is only reading. An EUI view can now be served as one cacheable render, keep the tree the client already fetched, carry a live island inside a cached page, and resume a dropped socket with only the batches it missed.", "tag": "Deep Dive", "image": "eui-islands-and-resumable-sessions.svg", "publish_on": "2026-09-28"},
        {"slug": "security-audit-41-findings", "file": "docs/blog/security-audit-41-findings.md", "desc": "A static audit of Soli's server, builtins, ORM and interpreter came back with 41 findings — most of them checks that existed but weren't where the request actually went. What changed, the leaks fixed in long-lived workers, the advisories no longer waived, and what to set after upgrading.", "tag": "Security", "image": "security-audit-41-findings.svg", "publish_on": "2026-09-27"},
        {"slug": "eui-tailwind-classes", "file": "docs/blog/eui-tailwind-classes.md", "desc": "tw() in the scaffolded EUI catalogue takes Tailwind class strings and turns them into styles for a native client, with nothing new on the wire. Colours become theme roles, sm:–2xl: resolve on the server, a class with no equivalent raises by name, and the half steps were added because they were the most frequent refusals.", "tag": "Feature", "image": "eui-tailwind-classes.svg", "publish_on": "2026-09-26"},
        {"slug": "error-tracking", "file": "docs/blog/error-tracking.md", "desc": "Every request that ends in a 500 is grouped by fingerprint into a table in your app's own database and shown at /__soli/errors — no SDK, no third party. How the fingerprint is built, why recording never blocks a request, how redaction and the admin gate work, and what it doesn't do compared with Sentry.", "tag": "Feature", "image": "error-tracking.svg", "publish_on": "2026-09-25"},
        {"slug": "eui-notes-app", "file": "docs/blog/eui-notes-app.md", "desc": "Build a notes app that opens in a native window — no HTML, no CSS, no JavaScript. An EUI component is a handler and a view that returns plain Soli hashes; the runtime diffs the tree and sends the difference. Motion, local handlers, assets, capabilities and scenes.", "tag": "Tutorial", "image": "eui-notes-app.svg"},
        {"slug": "whats-unreleased", "file": "docs/blog/whats-unreleased.md", "desc": "A tour of everything that shipped in v2.0.0: SQL as a real backend, in-process jobs, LiveView rooms and hardening, unless/end, and auth that no longer helps attackers.", "tag": "Guide", "image": "whats-unreleased.svg"},
        {"slug": "stripe-checkout", "file": "docs/blog/stripe-checkout.md", "desc": "Take payments with Stripe Checkout in a Soli app — no generator. Create a session, send the buyer to Stripe, and mark the order paid only after a signed webhook. HTTP.post, Crypto.hmac, skip_csrf.", "tag": "Tutorial", "image": "stripe-checkout.svg"},
        {"slug": "liveview-desk", "file": "docs/blog/liveview-desk.md", "desc": "A live Field Desk on the page: nested live_component assigns, soli-upload, in-socket tabs, debounce, click-away, hooks, JS commands, and the hash .where / jobs snippets you would ship with it.", "tag": "Tutorial", "image": "liveview-desk.svg"},
        {"slug": "multi-database", "file": "docs/blog/multi-database.md", "desc": "Named connections in config/database.toml and per-model connection \"name\" — SoliDB for the product core, Postgres or MySQL where the data already lives. SQL document adapters, honest capability matrix, and hard errors on cross-connection includes.", "tag": "Architecture", "image": "multi-database.svg"},
        {"slug": "serve-any-folder", "file": "docs/blog/serve-any-folder.md", "desc": "soli serve now works on any folder, not just Soli apps — files off disk, Markdown rendered, templates executed, a tree(1) sidebar per page. The design problem worth writing about: the same URL must return a viewer page to a click and raw bytes to an <img>, which Sec-Fetch-Dest answers cleanly. Plus offline-only highlighting via the language's own lexer, index-vs-README, loopback by default, and how a sidebar went from 594 KB to 182 KB a page.", "tag": "Feature", "image": "serve-any-folder.svg"},
        {"slug": "native-mobile", "file": "docs/blog/native-mobile.md", "desc": "Ship iOS and Android without rewriting the stack: thin WebView shells load your deployed Soli app, the native bridge covers open-app OS work, Push.deliver cascades to APNs/FCM when closed, and AppLinks deep-link into the web view — same models and templates, honest platform costs.", "tag": "Feature", "image": "native-mobile.svg"},
        {"slug": "desktop-build", "file": "docs/blog/desktop-build.md", "desc": "soli desktop build turns your Soli app into one double-clickable executable with an embedded encrypted app, a private SolidB, optional ref_* seed data, loopback launch tokens, cross-builds, and a path to signed OTA updates — plus an honest account of what local encryption does and does not protect.", "tag": "Feature", "image": "desktop-build.svg"},
        {"slug": "release-channels", "file": "docs/blog/release-channels.md", "desc": "How to operate a Soli auto-update release channel: signing keys, CDN layout, multi-platform builds, canary before stable, fix-forward rollbacks, key rotation, and what to monitor — the runbook the feature docs leave to ops.", "tag": "Guide", "image": "release-channels.svg"},
        {"slug": "browser-testing", "file": "docs/blog/browser-testing.md", "desc": "soli test --browser drives a real headless Chrome over the DevTools protocol, spoken by the soli binary itself — no Node, no npm, no Playwright. Real input events rather than element.click(), assertions that wait, and a cookie jar shared with your request specs so login() carries straight into visit().", "tag": "Feature", "image": "browser-testing.svg"},
        {"slug": "code-graph-rag", "file": "docs/blog/code-graph-rag.md", "desc": "soli graph build turns your project into a SolidB code graph with embeddings; soli graph query does semantic seed + edge expansion so agents get routes, callers, and views — not isolated snippets. Same database as Models, incremental sync, multi-language extractors.", "tag": "Feature", "image": "code-graph-rag.svg"},
        {"slug": "rag-product-discovery", "file": "docs/blog/rag-product-discovery.md", "desc": "Build retrieval-augmented product discovery in Soli: auto-embed on save, rank with .similar() and hybrid(), then ground answers with llm_generate — no separate vector DB or orchestration framework.", "tag": "Tutorial", "image": "rag-product-discovery.svg"},
        {"slug": "tamper-evident-ledgers", "file": "docs/blog/tamper-evident-ledgers.md", "desc": "Turn an audit log into a hash-chained, tamper-evident ledger with two builtins — Crypto.canonical_json for stable bytes and Crypto.merkle_root for a one-hash integrity proof. Catch silent edits, pin them to the exact record, and archive a compact daily commitment.", "tag": "Security", "image": "tamper-evident-ledgers.svg"},
        {"slug": "streaming-ai-progress", "file": "docs/blog/streaming-ai-progress.md", "desc": "Stream an AI agent's progress to the browser as it works — plan, tool calls, synthesis — with Server-Sent Events and a single sse() block. No WebSocket, no client framework, and the run stops if the user leaves.", "tag": "Tutorial", "image": "streaming-ai-progress.svg"},
        {"slug": "e2e-testing", "file": "docs/blog/e2e-testing.md", "desc": "Write realistic full-stack integration tests entirely in Soli using a built-in test server, clean BDD DSL, and coverage gates — no Playwright or Node required.", "tag": "Tutorial", "image": "e2e-testing.jpg"},
        {"slug": "scaffolds", "file": "docs/blog/scaffolds.md", "desc": "One command (`/soli-resource`) to go from concept to production-ready model, controller, views, migration, routes, and specs — with an intelligent pause for human judgment.", "tag": "Tutorial", "image": "scaffolds.jpg"},
        {"slug": "pattern-matching", "file": "docs/blog/pattern-matching.md", "desc": "Destructuring, guards, rest patterns, and exhaustive matching as a daily tool for controllers and data pipelines.", "tag": "Deep Dive", "image": "pattern-matching.jpg"},
        {"slug": "advanced-modeling", "file": "docs/blog/advanced-modeling.md", "desc": "Scopes, soft deletes, and transactions — the model layer features that let you express real business rules cleanly and safely.", "tag": "Tutorial", "image": "advanced-modeling.jpg"},
        {"slug": "ai-coding-agents", "file": "docs/blog/ai-coding-agents.md", "desc": "How Soli ships projects that are deliberately excellent environments for AI coding agents (Claude Code, Cursor, Aider) from the very first `soli new`.", "tag": "Philosophy", "image": "ai-coding-agents.jpg"},
        {"slug": "liveview", "file": "docs/blog/liveview.md", "desc": "Build real-time server-rendered UIs with Soli LiveView. A practical team presence + activity feed widget with ticks, simulated remote activity, and zero client-side JavaScript.", "tag": "Tutorial", "image": "liveview-activity.jpg"},
        {"slug": "background-jobs-and-cron", "file": "docs/blog/background-jobs-and-cron.md", "desc": "SolidB-backed queues, signed webhook callbacks, perform_later / perform_in, declarative cron, idempotency patterns, hot reload, and zero extra daemons — the complete background job system.", "tag": "Architecture", "image": "background-jobs-cron.jpg"},
        {"slug": "event-streaming-with-es", "file": "docs/blog/event-streaming-with-es.md", "desc": "Wire es — a single-binary, Kafka-shaped broker — to a Soli app with the built-in ES driver: produce events from a controller, make retries idempotent, drain them with a consumer group, and resume cleanly after a restart.", "tag": "Tutorial", "image": "event-streaming-es.jpg"},
        {"slug": "sendgrid-email-jobs", "file": "docs/blog/sendgrid-email-jobs.md", "desc": "Wrap the SendGrid v3 Messages API in a tiny Soli library, then hand delivery off to a SolidB-backed background job so the controller returns in milliseconds.", "tag": "Tutorial", "image": "sendgrid-jobs-flow.jpg"},
        {"slug": "htmx-datatable", "file": "docs/blog/htmx-datatable.md", "desc": "Build a full CRUD datatable — search, sort, pagination, inline edit, role select, status toggle, modal add, toast on save — with one model, one controller, two partials, and zero hand-written JavaScript.", "tag": "Tutorial", "image": "htmx-datatable.jpg"},
        {"slug": "similar-search", "file": "docs/blog/similar-search.md", "desc": "Add AI-native vector similarity search to any query chain with .similar(), ranking results by semantic relevance with cosine similarity.", "tag": "Tutorial", "image": "similar-search.jpg"},
        {"slug": "no-build-no-dependency", "file": "docs/blog/no-build-no-dependency.md", "desc": "Why Soli ships as a single binary with no package manager, no bundler, and no build step — and what that means for supply-chain security and operational simplicity.", "tag": "Philosophy", "image": "no-build-no-dependency.jpg"},
        {"slug": "password-validation", "file": "docs/blog/password-validation.md", "desc": "Enforce password character-class requirements on the server with .letters(), .mixed_case(), .numbers(), and .symbols(), and drive the HTML passwordrules attribute from the same validator chain.", "tag": "Security", "image": "password-validation.jpg"},
        {"slug": "web-push-notifications", "file": "docs/blog/web-push-notifications.md", "desc": "Drop the web-push Node module and send Web Push notifications natively from Soli with the four VAPID builtins (RFC 8291 / 8292).", "tag": "Tutorial", "image": "web-push-vapid.jpg"},
        {"slug": "dev-bar", "file": "docs/blog/dev-bar.md", "desc": "A look at Soli's new development bar: request timing, render breakdowns, SolidB queries, outgoing HTTP calls, N+1 detection, flamegraphs, and trace exports.", "tag": "Feature", "image": "dev-bar.png"},
        {"slug": "competing-with-big-frameworks", "file": "docs/blog/competing-with-big-frameworks.md", "desc": "How Soli competes with Rails, Laravel, Django, Next.js, and other mature frameworks by focusing on simplicity, coherence, and fast product development.", "tag": "Philosophy", "image": "competing-with-big-frameworks.jpg"},
        {"slug": "uploads-and-image-transforms", "file": "docs/blog/uploads-and-image-transforms.md", "desc": "Declare uploader(...) on the model, get show/create/destroy routes for free, drive image transforms (resize, crop, fit, blur, brightness, format) from URL query params.", "tag": "Feature", "image": "uploads-and-image-transforms.jpg"},
        {"slug": "spreadsheet-functions", "file": "docs/blog/spreadsheet-functions.md", "desc": "Parse and process CSV and Excel files with built-in spreadsheet functions.", "tag": "Tutorial", "image": "spreadsheet-functions.jpg"},
        {"slug": "totp-authentication", "file": "docs/blog/totp-authentication.md", "desc": "Add secure two-factor authentication to your SoliLang app with TOTP codes.", "tag": "Security", "image": "totp-auth.jpg"},
        {"slug": "github-oauth", "file": "docs/blog/github-oauth.md", "desc": "Full OAuth 2.0 flow with GitHub, sessions, JWT tokens, and security best practices.", "tag": "Tutorial", "image": "github-oauth.jpg"},
        {"slug": "benchmarks-reality", "file": "docs/blog/benchmarks-reality.md", "desc": "Why synthetic benchmarks are misleading and how oha provides better HTTP load testing.", "tag": "Deep Dive", "image": "benchmarks-reality.jpg"},
        {"slug": "google-oauth", "file": "docs/blog/google-oauth.md", "desc": "Learn how to add Google OAuth authentication to your SoliLang application.", "tag": "Tutorial", "image": "google-oauth.jpg"},
        {"slug": "htmx-integration", "file": "docs/blog/htmx-integration.md", "desc": "How HTMx brings simplicity to Soli web apps with server-rendered partials.", "tag": "Guide", "image": "htmx-integration.jpg"},
        {"slug": "soli-minimal-lang", "file": "docs/blog/soli-minimal-lang.md", "desc": "Why Soli is designed as a minimal, focused language for web development.", "tag": "Philosophy"}
    ]
end

def get_blog_posts()
    let posts = []

    for info in blog_manifest()
        let path = info["file"]
        next unless blog_visible?(info)

        if file_exists(path)
            let content = slurp(path)
            if content != nil and content != ""
                let title = extract_title(content)
                
                posts.push({
                    "slug": info["slug"],
                    "title": title,
                    "description": info["desc"],
                    "tag": info["tag"],
                    "tag_chip": tag_chip_class(info["tag"]),
                    "tag_gradient": tag_gradient_class(info["tag"]),
                    "image": info["image"] ?? null,
                    "publish_on": info["publish_on"] ?? null,
                    "scheduled_on": blog_scheduled_on(info)
                })
            end
        end
    end
    
    posts
end

# Scheduling: an entry with "publish_on": "YYYY-MM-DD" stays off the index and
# answers 404 until that day, so a week of posts can ship in one deploy. A local
# run (`soli serve www --dev` on localhost / *.localhost / 127.0.0.1) shows them
# anyway, badged with their date. Keyed on the Host rather than on --dev itself:
# production pins a soli (deploy-www.yml) that no dev-mode builtin can be assumed in.
# X-Forwarded-Host wins: in production soli-proxy dials the app on localhost, so
# Host is always local there and the public name only arrives in that header
# (the proxy overwrites any value a client sends).
def blog_preview?
    headers = req["headers"]
    authority = (headers["x-forwarded-host"] || headers["host"] || "").split(",")[0].trim()
    host = authority.split(":")[0].downcase()
    # `.test` is what the local soli-proxy serves dev sites on (soli.solisoft.test);
    # it is reserved by RFC 6761 and never resolves on the public internet.
    host == "localhost" or host.ends_with?(".localhost") or host.ends_with?(".test") or host == "127.0.0.1"
end

def blog_scheduled_on(info)
    publish_on = info["publish_on"]
    return null if publish_on.nil?
    return null if publish_on <= DateTime.now().format("%Y-%m-%d")
    publish_on
end

# A slug that is in the manifest but not yet visible — the show action 404s it.
def blog_scheduled_slug?(slug)
    blog_manifest().any?(fn(info) info["slug"] == slug and not blog_visible?(info))
end

def blog_visible?(info)
    blog_scheduled_on(info).nil? or blog_preview?
end

def extract_title(markdown)
    let lines = markdown.split("\n")
    for line in lines
        if len(line) > 2
            if line[0] == "#" and line[1] == " "
                return line.replace("# ", "")
            end
        end
    end
    "Blog Post"
end

def show
    let slug = req["params"]["slug"]
    
    if slug == nil or slug == ""
        return redirect("/docs/blog")
    end
    
    let path = "docs/blog/" + slug + ".md"
    let manifest_entry = find_blog_post(slug)
    let exists = file_exists(path)

    if not exists or blog_scheduled_slug?(slug)
        return render("blog/not_found", {
            "title": "Post not found",
            "layout": "layouts/docs",
            "breadcrumb_href": "/docs/blog",
            "recent_posts": get_blog_posts().take(3)
        }, {"status": 404})
    end
    
    let content = slurp(path)
    let html = Markdown.to_html(content)
    let title = extract_title(content)
    
    render("blog/show", {
        "title": title,
        "layout": "layouts/docs",
        "breadcrumb_href": "/docs/blog",
        "content": html,
        "slug": slug,
        "og_image": blog_og_image(slug),
        "og_image_size": blog_og_image_size(slug),
        "published_on": manifest_entry.nil? ? null : manifest_entry["publish_on"],
        "scheduled_on": manifest_entry.nil? ? null : manifest_entry["scheduled_on"],
        "og_description": blog_og_description(slug),
        "eui_build": slug == "pocket-garden-eui" ? blog_eui_build() : "none"
    })
end

# Which build of the EUI browser client is in public/eui, for the playable
# Pocket Garden: the version `xtask-web` stamps into the manifest beside the
# module, put on the script URL because the module is served `immutable` for
# a year. "none" when no client is built (a checkout that never ran the
# deploy's build step); the page still shows its poster.
def blog_eui_build
    raw = slurp("public/eui/manifest.json") rescue nil
    return "none" if raw.nil?

    parsed = JSON.parse(raw) rescue nil
    parsed.nil? ? "none" : (parsed["version"] ?? "none")
end

# Open Graph helpers: reuse a post's card image + description for link previews
# (Slack, X, LinkedIn, iMessage, …). Early-return lookups keep the values out of
# block-scoped assignment.
def find_blog_post(slug)
    for post in get_blog_posts()
        return post if post["slug"] == slug
    end
    return null
end

# X, LinkedIn, Facebook and Slack don't render an SVG og:image, so a post whose
# hero is an SVG shares a 1200x630 PNG render of it from images/blog/og/<slug>.png
# (rsvg-convert -w 1200 <slug>.svg | magick - -gravity center -crop 1200x630+0+0).
def blog_og_image(slug)
    let post = find_blog_post(slug)
    return null if post == null
    return "https://soli.solisoft.net/images/blog/og/" + slug + ".png" if blog_og_card?(slug)
    return null if post["image"] == null
    return "https://soli.solisoft.net/images/blog/" + post["image"]
end

def blog_og_card?(slug)
    file_exists("public/images/blog/og/" + slug + ".png")
end

def blog_og_image_size(slug)
    return {"width": 1200, "height": 630} if blog_og_card?(slug)
    null
end

def blog_og_description(slug)
    let post = find_blog_post(slug)
    return "" if post == null
    return post["description"]
end