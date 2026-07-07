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

def get_blog_posts()
    let posts = []

    # Simple list - ordered manually (newest first)
    let blog_info = [
        {"slug": "tamper-evident-ledgers", "file": "docs/blog/tamper-evident-ledgers.md", "desc": "Turn an audit log into a hash-chained, tamper-evident ledger with two builtins — Crypto.canonical_json for stable bytes and Crypto.merkle_root for a one-hash integrity proof. Catch silent edits, pin them to the exact record, and archive a compact daily commitment.", "tag": "Security", "image": "tamper-evident-ledgers.svg"},
        {"slug": "streaming-ai-progress", "file": "docs/blog/streaming-ai-progress.md", "desc": "Stream an AI agent's progress to the browser as it works — plan, tool calls, synthesis — with Server-Sent Events and a single sse() block. No WebSocket, no client framework, and the run stops if the user leaves.", "tag": "Tutorial", "image": "streaming-ai-progress.svg"},
        {"slug": "e2e-testing", "file": "docs/blog/e2e-testing.md", "desc": "Write realistic full-stack integration tests entirely in Soli using a built-in test server, clean BDD DSL, and coverage gates — no Playwright or Node required.", "tag": "Tutorial", "image": "e2e-testing.jpg"},
        {"slug": "scaffolds", "file": "docs/blog/scaffolds.md", "desc": "One command (`/soli-resource`) to go from concept to production-ready model, controller, views, migration, routes, and specs — with an intelligent pause for human judgment.", "tag": "Tutorial", "image": "scaffolds.jpg"},
        {"slug": "pattern-matching", "file": "docs/blog/pattern-matching.md", "desc": "Destructuring, guards, rest patterns, and exhaustive matching as a daily tool for controllers and data pipelines.", "tag": "Deep Dive", "image": "pattern-matching.jpg"},
        {"slug": "advanced-modeling", "file": "docs/blog/advanced-modeling.md", "desc": "Scopes, soft deletes, and transactions — the model layer features that let you express real business rules cleanly and safely.", "tag": "Tutorial", "image": "advanced-modeling.jpg"},
        {"slug": "ai-coding-agents", "file": "docs/blog/ai-coding-agents.md", "desc": "How Soli ships projects that are deliberately excellent environments for AI coding agents (Claude Code, Cursor, Aider) from the very first `soli new`.", "tag": "Philosophy", "image": "ai-coding-agents.jpg"},
        {"slug": "liveview", "file": "docs/blog/liveview.md", "desc": "Build real-time server-rendered UIs with Soli LiveView. A practical team presence + activity feed widget with ticks, simulated remote activity, and zero client-side JavaScript.", "tag": "Tutorial", "image": "liveview-activity.jpg"},
        {"slug": "background-jobs-and-cron", "file": "docs/blog/background-jobs-and-cron.md", "desc": "SolidB-backed queues, signed webhook callbacks, perform_later / perform_in, declarative cron, idempotency patterns, hot reload, and zero extra daemons — the complete background job system.", "tag": "Architecture", "image": "background-jobs-cron.jpg"},
        {"slug": "event-streaming-with-es", "file": "docs/blog/event-streaming-with-es.md", "desc": "Wire es — a single-binary, HTTP+JSON, Kafka-shaped broker — to a Soli app end-to-end: produce events from a controller, drain them with a consumer-group-backed background job, and resume cleanly after a restart.", "tag": "Tutorial", "image": "event-streaming-es.jpg"},
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
    
    for info in blog_info
        let path = info["file"]
        
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
                    "image": info["image"] ?? null
                })
            end
        end
    end
    
    posts
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
    let exists = file_exists(path)
    
    if not exists
        return {
            "status": 404,
            "headers": {"Content-Type": "text/html"},
            "body": "<h1>404 - Blog post not found</h1>"
        }
    end
    
    let content = slurp(path)
    let html = Markdown.to_html(content)
    let title = extract_title(content)
    
    render("blog/show", {
        "title": title,
        "layout": "layouts/docs",
        "breadcrumb_href": "/docs/blog",
        "content": html,
        "slug": slug
    })
end