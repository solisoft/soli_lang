# Blog Controller
# Handles displaying blog posts (markdown content)

fn index(req)
    let posts = get_blog_posts()
    
    render("blog/index", {
        "title": "Blog",
        "layout": "layouts/docs",
        "posts": posts
    })
end

fn get_blog_posts()
    let posts = []
    
    # Simple list - ordered manually (newest first)
    let blog_info = [
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
        "content": html,
        "slug": slug
    })
end