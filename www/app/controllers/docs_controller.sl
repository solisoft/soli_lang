# Documentation Controller
# Handles displaying documentation pages

def index
    redirect("/docs/getting-started/introduction")
end

# Helper to render docs pages with consistent context
def render_docs(view, title, section, subsection, hide_toc = false)
    render(view, {
        "title": title,
        "section": section,
        "subsection": subsection,
        "hide_toc": hide_toc,
        "layout": "layouts/docs"
    })
end

# ============================================================================
# Getting Started
# ============================================================================

def getting_started_introduction
    render_docs("docs/getting-started/introduction", "Introduction", "getting_started", "introduction")
end

def getting_started_installation
    render_docs("docs/getting-started/installation", "Installation", "getting_started", "installation")
end

def getting_started_configuration
    render_docs("docs/getting-started/configuration", "Configuration", "getting_started", "configuration")
end

def getting_started_comparison
    render_docs("docs/getting-started/comparison", "How Soli Compares", "getting_started", "comparison")
end

def getting_started_changelog
    render_docs("docs/getting-started/changelog", "Changelog", "getting_started", "changelog")
end

# ============================================================================
# Core Concepts
# ============================================================================

def core_concepts_routing
    render_docs("docs/core-concepts/routing", "Routing", "core_concepts", "routing")
end

def core_concepts_controllers
    render_docs("docs/core-concepts/controllers", "Controllers", "core_concepts", "controllers")
end

def core_concepts_middleware
    render_docs("docs/core-concepts/middleware", "Middleware", "core_concepts", "middleware")
end

def core_concepts_views
    render_docs("docs/core-concepts/views", "Views", "core_concepts", "views")
end

def core_concepts_forms
    render_docs("docs/core-concepts/forms", "Forms & CSRF", "core_concepts", "forms")
end

def core_concepts_websockets
    render_docs("docs/core-concepts/websockets", "WebSockets", "core_concepts", "websockets")
end

def core_concepts_streaming
    render_docs("docs/core-concepts/streaming", "Streaming & SSE", "core_concepts", "streaming")
end

def core_concepts_liveview
    render_docs("docs/core-concepts/liveview", "Live View", "core_concepts", "liveview")
end

def core_concepts_client_interactivity
    render_docs("docs/core-concepts/client-interactivity", "Client Interactivity", "core_concepts", "client_interactivity")
end

def core_concepts_i18n
    render_docs("docs/core-concepts/i18n", "Internationalization", "core_concepts", "i18n")
end

def core_concepts_request_params
    render_docs("docs/core-concepts/request-params", "Request Parameters", "core_concepts", "request_params")
end

def core_concepts_error_pages
    render_docs("docs/core-concepts/error-pages", "Error Pages", "core_concepts", "error_pages")
end

def core_concepts_engines
    render_docs("docs/core-concepts/engines", "Engines", "core_concepts", "engines")
end

def core_concepts_feature_flags
    render_docs("docs/core-concepts/feature-flags", "Feature Flags", "core_concepts", "feature_flags")
end

# ============================================================================
# Database
# ============================================================================

def database_configuration
    render_docs("docs/database/configuration", "Database Configuration", "database", "configuration")
end

def database_models
    render_docs("docs/database/models", "Models & ORM", "database", "models")
end

def database_query_builder
    render_docs("docs/database/query-builder", "Query Builder", "database", "query_builder")
end

def database_relationships
    render_docs("docs/database/relationships", "Relationships", "database", "relationships")
end

def database_validations
    render_docs("docs/database/validations", "Validations & Callbacks", "database", "validations")
end

def database_state_machines
    render_docs("docs/database/state-machines", "State Machines", "database", "state_machines")
end

def database_finders
    render_docs("docs/database/finders", "Finders & Aggregations", "database", "finders")
end

def database_analytics
    render_docs("docs/database/analytics", "Analytics & Columnar Stores", "database", "analytics")
end

def database_search
    render_docs("docs/database/search", "Search: Vector, Fulltext & Geo", "database", "search")
end

def database_advanced
    render_docs("docs/database/advanced", "Advanced Features", "database", "advanced")
end

def database_migrations
    render_docs("docs/database/migrations", "Migrations", "database", "migrations")
end

# ============================================================================
# Security
# ============================================================================

def security_authentication
    render_docs("docs/security/authentication", "Authentication with JWT", "security", "authentication")
end

def security_sessions
    render_docs("docs/security/sessions", "Sessions", "security", "sessions")
end

def security_authorization
    render_docs("docs/security/authorization", "Authorization & Policies", "security", "authorization")
end

def security_oidc_provider
    render_docs("docs/security/oidc-provider", "OpenID Connect Provider", "security", "oidc_provider")
end

# Development Tools
# ============================================================================

def development_tools_live_reload
    render_docs("docs/development-tools/live-reload", "Live Reload", "development_tools", "live_reload")
end

def development_tools_debugging
    render_docs("docs/development-tools/debugging", "Debugging", "development_tools", "debugging")
end

def development_tools_scaffold
    render_docs("docs/development-tools/scaffold", "Scaffold Generator", "development_tools", "scaffold")
end

def development_tools_deploy
    render_docs("docs/development-tools/deploy", "Deploy", "development_tools", "deploy")
end

def development_tools_desktop
    render_docs("docs/development-tools/desktop", "Desktop Apps", "development_tools", "desktop")
end

def development_tools_native_bridge
    render_docs("docs/development-tools/native-bridge", "Native Bridge", "development_tools", "native-bridge")
end

def native_notifications
    render_docs("docs/native/notifications", "Notifications", "native", "notifications")
end

def native_camera
    render_docs("docs/native/camera", "Camera & Microphone", "native", "camera")
end

def native_scanning
    render_docs("docs/native/scanning", "Barcode & QR Scanning", "native", "scanning")
end

def native_geolocation
    render_docs("docs/native/geolocation", "Geolocation", "native", "geolocation")
end

def native_device
    render_docs("docs/native/device", "Device Capabilities", "native", "device")
end

def native_deep_links
    render_docs("docs/native/deep-links", "Deep Links", "native", "deep-links")
end

def native_push_apple
    render_docs("docs/native/push-apple", "Apple Push (APNs)", "native", "push-apple")
end

def native_push_android
    render_docs("docs/native/push-android", "Android Push (FCM)", "native", "push-android")
end

def development_tools_editor_integration
    render_docs(
        "docs/development-tools/editor-integration",
        "Editor Integration",
        "development_tools",
        "editor_integration"
    )
end

def development_tools_formatting
    render_docs("docs/development-tools/formatting", "Formatting", "development_tools", "formatting")
end

def development_tools_ai_agents
    render_docs(
        "docs/development-tools/ai-agents",
        "AI Agents",
        "development_tools",
        "ai_agents"
    )
end

def development_tools_linting
    render_docs("docs/development-tools/linting", "Linting", "development_tools", "linting")
end

def development_tools_graph
    render_docs("docs/development-tools/graph", "Code Graph", "development_tools", "graph")
end

# ============================================================================
# Language Reference
# ============================================================================

def language_index
    render_docs("docs/language/index", "Soli Language Reference", "language", "index")
end

def language_variables_types
    render_docs("docs/language/variables-types", "Variables & Types", "language", "variables_types")
end

def language_operators
    render_docs("docs/language/operators", "Operators", "language", "operators")
end

def language_control_flow
    render_docs("docs/language/control-flow", "Control Flow", "language", "control_flow")
end

def language_error_handling
    render_docs("docs/language/error-handling", "Error Handling", "language", "error_handling")
end

def language_functions
    render_docs("docs/language/functions", "Functions", "language", "functions")
end

def language_strings
    render_docs("docs/language/strings", "Strings", "language", "strings")
end

def language_arrays
    render_docs("docs/language/arrays", "Arrays", "language", "arrays")
end

def language_hashes
    render_docs("docs/language/hashes", "Hashes", "language", "hashes")
end

def language_collections
    render_docs("docs/language/collections", "Collections", "language", "collections")
end

def language_classes_oop
    render_docs("docs/language/classes-oop", "Classes & OOP", "language", "classes_oop")
end

def language_pattern_matching
    render_docs("docs/language/pattern-matching", "Pattern Matching", "language", "pattern_matching")
end

def language_enums
    render_docs("docs/language/enums", "Enums", "language", "enums")
end

def language_pipeline_operator
    render_docs("docs/language/pipeline-operator", "Pipeline Operator", "language", "pipeline_operator")
end

def language_modules
    render_docs("docs/language/modules", "Modules", "language", "modules")
end

def language_integers
    render_docs("docs/language/integers", "Integers", "language", "integers")
end

def language_floats
    render_docs("docs/language/floats", "Floats", "language", "floats")
end

def language_booleans
    render_docs("docs/language/booleans", "Booleans", "language", "booleans")
end

def language_null
    render_docs("docs/language/null", "Null", "language", "null")
end

def language_decimal
    render_docs("docs/language/decimal", "Decimal", "language", "decimal")
end

def language_symbols
    render_docs("docs/language/symbols", "Symbols", "language", "symbols")
end

def language_blocks
    render_docs("docs/language/blocks", "Block Syntax", "language", "blocks")
end

def language_metaprogramming
    render_docs("docs/language/metaprogramming", "Metaprogramming", "language", "metaprogramming")
end

# ============================================================================
# Builtins Reference
# ============================================================================

def builtins_index
    render_docs("docs/builtins/index", "Built-in Functions", "builtins", "index")
end

def builtins_core
    render_docs("docs/builtins/core", "Core Functions", "builtins", "core")
end

def builtins_system
    render_docs("docs/builtins/system", "System Functions", "builtins", "system")
end

def builtins_http
    render_docs("docs/builtins/http", "HTTP Functions", "builtins", "http")
end

def builtins_s3
    render_docs("docs/builtins/s3", "S3 Functions", "builtins", "s3")
end

def builtins_json
    render_docs("docs/builtins/json", "JSON Functions", "builtins", "json")
end

def builtins_ai
    render_docs("docs/builtins/ai", "AI Functions", "builtins", "ai")
end

def builtins_crypto
    render_docs("docs/builtins/crypto", "Crypto Functions", "builtins", "crypto")
end

def builtins_jwt
    render_docs("docs/builtins/jwt", "JWT Functions", "builtins", "jwt")
end

def builtins_xml_signatures
    render_docs("docs/builtins/xml-signatures", "XML Signatures & Keys", "builtins", "xml-signatures")
end

def builtins_vapid
    render_docs("docs/builtins/vapid", "VAPID / Web Push Functions", "builtins", "vapid")
end

def builtins_regex
    render_docs("docs/builtins/regex", "Regex Functions", "builtins", "regex")
end

def builtins_env
    render_docs("docs/builtins/env", "Environment Functions", "builtins", "env")
end

def builtins_datetime
    render_docs("docs/builtins/datetime", "DateTime", "builtins", "datetime")
end

def builtins_duration
    render_docs("docs/builtins/duration", "Duration", "builtins", "duration")
end

def builtins_validation
    render_docs("docs/builtins/validation", "Validation Functions", "builtins", "validation")
end

def builtins_session
    render_docs("docs/builtins/session", "Session Functions", "builtins", "session")
end

def builtins_jobs
    render_docs("docs/builtins/jobs", "Jobs & Cron", "builtins", "jobs")
end

def builtins_testing
    render_docs("docs/builtins/testing", "Testing Functions", "builtins", "testing")
end

def builtins_i18n
    render_docs("docs/builtins/i18n", "I18n Functions", "builtins", "i18n")
end

def builtins_cache
    render_docs("docs/builtins/cache", "Cache Functions", "builtins", "cache")
end

def builtins_kv
    render_docs("docs/builtins/kv", "KV Store", "builtins", "kv")
end

def builtins_solidb
    render_docs("docs/builtins/solidb", "Solidb", "builtins", "solidb")
end

def builtins_rate_limit
    render_docs("docs/builtins/rate-limit", "Rate Limiting Functions", "builtins", "rate_limit")
end

def builtins_security_headers
    render_docs("docs/builtins/security-headers", "Security Headers Functions", "builtins", "security_headers")
end

def builtins_hardening
    render_docs("docs/builtins/hardening", "Server Hardening", "builtins", "hardening")
end

def builtins_upload
    render_docs("docs/builtins/upload", "File Upload Functions", "builtins", "upload")
end

def builtins_soap
    render_docs("docs/builtins/soap", "SOAP Class", "builtins", "soap")
end

def builtins_pop3
    render_docs("docs/builtins/pop3", "POP3 Email Class", "builtins", "pop3")
end

def builtins_imap
    render_docs("docs/builtins/imap", "IMAP Email Class", "builtins", "imap")
end

def builtins_mailer
    render_docs("docs/builtins/mailer", "Mailer", "builtins", "mailer")
end

def builtins_markdown
    render_docs("docs/builtins/markdown", "Markdown Class", "builtins", "markdown")
end

def builtins_image
    render_docs("docs/builtins/image", "Image Class", "builtins", "image")
end

def builtins_pdf
    render_docs("docs/builtins/pdf", "PDF & Factur-X", "builtins", "pdf")
end

def pdf_templates
    render_docs("docs/pdf_templates", "Invoice & Quote Templates", "builtins", "pdf", true)
end

def pdf_editor
    render_docs("docs/pdf_editor", "Layout Editor", "builtins", "pdf", true)
end

# The studio is a full-screen canvas, so it renders without the docs chrome.
def pdf_studio
    render("docs/pdf_studio", {}, {"layout": false})
end

# Where each element landed, so the studio can hit-test the rendered page.
# A flowing element's position depends on everything before it, so only the
# layout engine can answer this — the editor cannot compute it.
def pdf_studio_layout
    let template = params["template"] ?? ""
    let data = params["data"] ?? "{}"
    try
        let boxes = pdf_layout_map(template, data, {"fetch_images": false, "font_dirs": ["font"]})
        return {
            "status": 200,
            "headers": {"Content-Type": "application/json"},
            "body": boxes.to_json()
        }
    catch e
        return {"status": 400, "headers": {"Content-Type": "text/plain"}, "body": str(e)}
    end
end

def pdf_playground
    render_docs("docs/pdf_playground", "PDF Playground", "builtins", "pdf", true)
end

def pdf_playground_render
    let markdown = params["markdown"] ?? ""
    try
        let t0 = clock()
        let pdf = null
        if markdown.blank?
            let template = params["template"] ?? ""
            let data = params["data"] ?? "{}"
            pdf = pdf_render(template, data, { "fetch_images": false, "font_dirs": ["font"] })
        else
            # Markdown → PDF sample: fold the Markdown into the layout engine.
            pdf = pdf_from_markdown(markdown, { "font_dirs": ["font"] })
        end
        let engine_ms = ((clock() - t0) * 1000).round()
        let headers = { "Content-Type": "text/plain", "X-Render-Ms": str(engine_ms) }
        return { "status": 200, "headers": headers, "body": pdf }
    catch e
        return { "status": 400, "headers": {"Content-Type": "text/plain"}, "body": str(e) }
    end
end

def builtins_file
    render_docs("docs/builtins/file", "File Class", "builtins", "file")
end

def builtins_spreadsheet
    render_docs("docs/builtins/spreadsheet", "Spreadsheet Functions", "builtins", "spreadsheet")
end

# /docs/builtins/websocket merged into /docs/core-concepts/websockets
def builtins_websocket
    redirect("/docs/core-concepts/websockets")
end

# ============================================================================
# Utility
# ============================================================================

def utility_base64
    render_docs("docs/utility/base64", "Base64 Encoding", "utility", "base64")
end

def utility_encoding
    render_docs("docs/utility/encoding", "Character Encodings", "utility", "encoding")
end

# ============================================================================ Testing
# ============================================================================

def testing
    render_docs("docs/core-concepts/testing", "Testing", "testing", "testing")
end

def testing_browser
    render_docs(
        "docs/core-concepts/testing-browser",
        "Browser Testing",
        "testing",
        "testing_browser"
    )
end

def testing_quick_reference
    render_docs(
        "docs/core-concepts/testing-quick-reference",
        "Testing Quick Reference",
        "testing",
        "testing_quick_reference"
    )
end

# ============================================================================
# Backward Compatibility Redirects
# ============================================================================

def redirect_introduction
    redirect("/docs/getting-started/introduction")
end

def redirect_installation
    redirect("/docs/getting-started/installation")
end

def redirect_configuration
    redirect("/docs/getting-started/configuration")
end

def redirect_routing
    redirect("/docs/core-concepts/routing")
end

def redirect_controllers
    redirect("/docs/core-concepts/controllers")
end

def redirect_middleware
    redirect("/docs/core-concepts/middleware")
end

def redirect_views
    redirect("/docs/core-concepts/views")
end

def redirect_websockets
    redirect("/docs/core-concepts/websockets")
end

def redirect_liveview
    redirect("/docs/core-concepts/liveview")
end

def redirect_i18n
    redirect("/docs/core-concepts/i18n")
end

def redirect_request_params
    redirect("/docs/core-concepts/request-params")
end

def redirect_error_pages
    redirect("/docs/core-concepts/error-pages")
end

def redirect_database
    redirect("/docs/database/configuration")
end

def redirect_models
    redirect("/docs/database/models")
end

def redirect_migrations
    redirect("/docs/database/migrations")
end

def redirect_authentication
    redirect("/docs/security/authentication")
end

def redirect_sessions
    redirect("/docs/security/sessions")
end

def redirect_validation
    redirect("/docs/builtins/validation")
end

def redirect_live_reload
    redirect("/docs/development-tools/live-reload")
end

def redirect_debugging
    redirect("/docs/development-tools/debugging")
end

def redirect_scaffold
    redirect("/docs/development-tools/scaffold")
end

def redirect_soli_language
    redirect("/docs/language")
end

# ============================================================================
# Core Concepts - State Machines
# ============================================================================

def core_concepts_state_machines
    render_docs("docs/core-concepts/state-machines", "State Machines", "core_concepts", "state_machines")
end

# ============================================================================
# Blog
# ============================================================================

def blog_index
    render_docs("docs/blog/index", "Blog", "blog", "index")
end

def blog_google_oauth
    render_docs("docs/blog/google-oauth", "Implementing Google OAuth in SoliLang", "blog", "google_oauth")
end
