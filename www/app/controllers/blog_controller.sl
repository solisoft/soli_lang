# Blog Controller
# Handles displaying blog posts (markdown content)

fn index(req)
    let posts = get_blog_posts()
    
    render("blog/index", {
        "title": "Blog",
        "layout": "layouts/docs",
        "breadcrumb_href": "/docs/blog",
        "posts": posts
    })
end

fn get_blog_posts()
    let posts = []
    
    # Simple list - ordered manually (newest first)
    let blog_info = [
        {"slug": "password-validation", "file": "docs/blog/password-validation.md", "desc": "Enforce password character-class requirements on the server with .letters(), .mixed_case(), .numbers(), and .symbols(), and drive the HTML passwordrules attribute from the same validator chain.", "tag": "Security"},
        {"slug": "web-push-notifications", "file": "docs/blog/web-push-notifications.md", "desc": "Drop the web-push Node module and send Web Push notifications natively from Soli with the four VAPID builtins (RFC 8291 / 8292).", "tag": "Tutorial"},
        {"slug": "dev-bar", "file": "docs/blog/dev-bar.md", "desc": "A look at Soli's new development bar: request timing, render breakdowns, SolidB queries, outgoing HTTP calls, N+1 detection, flamegraphs, and trace exports.", "tag": "Feature"},
        {"slug": "competing-with-big-frameworks", "file": "docs/blog/competing-with-big-frameworks.md", "desc": "How Soli competes with Rails, Laravel, Django, Next.js, and other mature frameworks by focusing on simplicity, coherence, and fast product development.", "tag": "Philosophy"},
        {"slug": "uploads-and-image-transforms", "file": "docs/blog/uploads-and-image-transforms.md", "desc": "Declare uploader(...) on the model, get show/create/destroy routes for free, drive image transforms (resize, crop, fit, blur, brightness, format) from URL query params.", "tag": "Feature"},
        {"slug": "spreadsheet-functions", "file": "docs/blog/spreadsheet-functions.md", "desc": "Parse and process CSV and Excel files with built-in spreadsheet functions.", "tag": "Tutorial"},
        {"slug": "totp-authentication", "file": "docs/blog/totp-authentication.md", "desc": "Add secure two-factor authentication to your SoliLang app with TOTP codes.", "tag": "Security"},
        {"slug": "github-oauth", "file": "docs/blog/github-oauth.md", "desc": "Full OAuth 2.0 flow with GitHub, sessions, JWT tokens, and security best practices.", "tag": "Tutorial"},
        {"slug": "benchmarks-reality", "file": "docs/blog/benchmarks-reality.md", "desc": "Why synthetic benchmarks are misleading and how oha provides better HTTP load testing.", "tag": "Deep Dive"},
        {"slug": "google-oauth", "file": "docs/blog/google-oauth.md", "desc": "Learn how to add Google OAuth authentication to your SoliLang application.", "tag": "Tutorial"},
        {"slug": "htmx-integration", "file": "docs/blog/htmx-integration.md", "desc": "How HTMx brings simplicity to Soli web apps with server-rendered partials.", "tag": "Guide"},
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
                    "tag": info["tag"]
                })
            end
        end
    end
    
    posts
end

fn extract_title(markdown)
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

fn show(req)
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