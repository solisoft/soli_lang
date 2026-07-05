# Documentation Controller
# Handles displaying documentation pages

fn index
    redirect("/docs/getting-started/introduction")
end

# Helper to render docs pages with consistent context
fn render_docs(view, title, section, subsection, hide_toc = false)
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

fn getting_started_introduction
    render_docs("docs/getting-started/introduction", "Introduction", "getting_started", "introduction")
end

fn getting_started_installation
    render_docs("docs/getting-started/installation", "Installation", "getting_started", "installation")
end

fn getting_started_configuration
    render_docs("docs/getting-started/configuration", "Configuration", "getting_started", "configuration")
end

fn getting_started_comparison
    render_docs("docs/getting-started/comparison", "How Soli Compares", "getting_started", "comparison")
end

# ============================================================================
# Core Concepts
# ============================================================================

fn core_concepts_routing
    render_docs("docs/core-concepts/routing", "Routing", "core_concepts", "routing")
end

fn core_concepts_controllers
    render_docs("docs/core-concepts/controllers", "Controllers", "core_concepts", "controllers")
end

fn core_concepts_middleware
    render_docs("docs/core-concepts/middleware", "Middleware", "core_concepts", "middleware")
end

fn core_concepts_views
    render_docs("docs/core-concepts/views", "Views", "core_concepts", "views")
end

fn core_concepts_websockets
    render_docs("docs/core-concepts/websockets", "WebSockets", "core_concepts", "websockets")
end

fn core_concepts_streaming
    render_docs("docs/core-concepts/streaming", "Streaming & SSE", "core_concepts", "streaming")
end

fn core_concepts_liveview
    render_docs("docs/core-concepts/liveview", "Live View", "core_concepts", "liveview")
end

fn core_concepts_client_interactivity
    render_docs("docs/core-concepts/client-interactivity", "Client Interactivity", "core_concepts", "client_interactivity")
end

fn core_concepts_i18n
    render_docs("docs/core-concepts/i18n", "Internationalization", "core_concepts", "i18n")
end

fn core_concepts_request_params
    render_docs("docs/core-concepts/request-params", "Request Parameters", "core_concepts", "request_params")
end

fn core_concepts_error_pages
    render_docs("docs/core-concepts/error-pages", "Error Pages", "core_concepts", "error_pages")
end

fn core_concepts_engines
    render_docs("docs/core-concepts/engines", "Engines", "core_concepts", "engines")
end

fn core_concepts_feature_flags
    render_docs("docs/core-concepts/feature-flags", "Feature Flags", "core_concepts", "feature_flags")
end

# ============================================================================
# Database
# ============================================================================

fn database_configuration
    render_docs("docs/database/configuration", "Database Configuration", "database", "configuration")
end

fn database_models
    render_docs("docs/database/models", "Models & ORM", "database", "models")
end

fn database_query_builder
    render_docs("docs/database/query-builder", "Query Builder", "database", "query_builder")
end

fn database_relationships
    render_docs("docs/database/relationships", "Relationships", "database", "relationships")
end

fn database_validations
    render_docs("docs/database/validations", "Validations & Callbacks", "database", "validations")
end

fn database_state_machines
    render_docs("docs/database/state-machines", "State Machines", "database", "state_machines")
end

fn database_finders
    render_docs("docs/database/finders", "Finders & Aggregations", "database", "finders")
end

fn database_advanced
    render_docs("docs/database/advanced", "Advanced Features", "database", "advanced")
end

fn database_migrations
    render_docs("docs/database/migrations", "Migrations", "database", "migrations")
end

# ============================================================================
# Security
# ============================================================================

fn security_authentication
    render_docs("docs/security/authentication", "Authentication with JWT", "security", "authentication")
end

fn security_sessions
    render_docs("docs/security/sessions", "Sessions", "security", "sessions")
end

fn security_authorization
    render_docs("docs/security/authorization", "Authorization & Policies", "security", "authorization")
end

# Development Tools
# ============================================================================

fn development_tools_live_reload
    render_docs("docs/development-tools/live-reload", "Live Reload", "development_tools", "live_reload")
end

fn development_tools_debugging
    render_docs("docs/development-tools/debugging", "Debugging", "development_tools", "debugging")
end

fn development_tools_scaffold
    render_docs("docs/development-tools/scaffold", "Scaffold Generator", "development_tools", "scaffold")
end

fn development_tools_deploy
    render_docs("docs/development-tools/deploy", "Deploy", "development_tools", "deploy")
end

fn development_tools_editor_integration
    render_docs(
        "docs/development-tools/editor-integration",
        "Editor Integration",
        "development_tools",
        "editor_integration"
    )
end

fn development_tools_formatting
    render_docs("docs/development-tools/formatting", "Formatting", "development_tools", "formatting")
end

fn development_tools_ai_agents
    render_docs(
        "docs/development-tools/ai-agents",
        "AI Agents",
        "development_tools",
        "ai_agents"
    )
end

fn development_tools_linting
    render_docs("docs/development-tools/linting", "Linting", "development_tools", "linting")
end

# ============================================================================
# Language Reference
# ============================================================================

fn language_index
    render_docs("docs/language/index", "Soli Language Reference", "language", "index")
end

fn language_variables_types
    render_docs("docs/language/variables-types", "Variables & Types", "language", "variables_types")
end

fn language_operators
    render_docs("docs/language/operators", "Operators", "language", "operators")
end

fn language_control_flow
    render_docs("docs/language/control-flow", "Control Flow", "language", "control_flow")
end

fn language_error_handling
    render_docs("docs/language/error-handling", "Error Handling", "language", "error_handling")
end

fn language_functions
    render_docs("docs/language/functions", "Functions", "language", "functions")
end

fn language_strings
    render_docs("docs/language/strings", "Strings", "language", "strings")
end

fn language_arrays
    render_docs("docs/language/arrays", "Arrays", "language", "arrays")
end

fn language_hashes
    render_docs("docs/language/hashes", "Hashes", "language", "hashes")
end

fn language_collections
    render_docs("docs/language/collections", "Collections", "language", "collections")
end

fn language_classes_oop
    render_docs("docs/language/classes-oop", "Classes & OOP", "language", "classes_oop")
end

fn language_pattern_matching
    render_docs("docs/language/pattern-matching", "Pattern Matching", "language", "pattern_matching")
end

fn language_enums
    render_docs("docs/language/enums", "Enums", "language", "enums")
end

fn language_pipeline_operator
    render_docs("docs/language/pipeline-operator", "Pipeline Operator", "language", "pipeline_operator")
end

fn language_modules
    render_docs("docs/language/modules", "Modules", "language", "modules")
end

fn language_integers
    render_docs("docs/language/integers", "Integers", "language", "integers")
end

fn language_floats
    render_docs("docs/language/floats", "Floats", "language", "floats")
end

fn language_booleans
    render_docs("docs/language/booleans", "Booleans", "language", "booleans")
end

fn language_null
    render_docs("docs/language/null", "Null", "language", "null")
end

fn language_decimal
    render_docs("docs/language/decimal", "Decimal", "language", "decimal")
end

fn language_symbols
    render_docs("docs/language/symbols", "Symbols", "language", "symbols")
end

fn language_blocks
    render_docs("docs/language/blocks", "Block Syntax", "language", "blocks")
end

fn language_metaprogramming
    render_docs("docs/language/metaprogramming", "Metaprogramming", "language", "metaprogramming")
end

# ============================================================================
# Builtins Reference
# ============================================================================

fn builtins_index
    render_docs("docs/builtins/index", "Built-in Functions", "builtins", "index")
end

fn builtins_core
    render_docs("docs/builtins/core", "Core Functions", "builtins", "core")
end

fn builtins_system
    render_docs("docs/builtins/system", "System Functions", "builtins", "system")
end

fn builtins_http
    render_docs("docs/builtins/http", "HTTP Functions", "builtins", "http")
end

fn builtins_s3
    render_docs("docs/builtins/s3", "S3 Functions", "builtins", "s3")
end

fn builtins_json
    render_docs("docs/builtins/json", "JSON Functions", "builtins", "json")
end

fn builtins_crypto
    render_docs("docs/builtins/crypto", "Crypto Functions", "builtins", "crypto")
end

fn builtins_jwt
    render_docs("docs/builtins/jwt", "JWT Functions", "builtins", "jwt")
end

fn builtins_vapid
    render_docs("docs/builtins/vapid", "VAPID / Web Push Functions", "builtins", "vapid")
end

fn builtins_regex
    render_docs("docs/builtins/regex", "Regex Functions", "builtins", "regex")
end

fn builtins_env
    render_docs("docs/builtins/env", "Environment Functions", "builtins", "env")
end

fn builtins_datetime
    render_docs("docs/builtins/datetime", "DateTime", "builtins", "datetime")
end

fn builtins_duration
    render_docs("docs/builtins/duration", "Duration", "builtins", "duration")
end

fn builtins_validation
    render_docs("docs/builtins/validation", "Validation Functions", "builtins", "validation")
end

fn builtins_session
    render_docs("docs/builtins/session", "Session Functions", "builtins", "session")
end

fn builtins_jobs
    render_docs("docs/builtins/jobs", "Jobs & Cron", "builtins", "jobs")
end

fn builtins_testing
    render_docs("docs/builtins/testing", "Testing Functions", "builtins", "testing")
end

fn builtins_i18n
    render_docs("docs/builtins/i18n", "I18n Functions", "builtins", "i18n")
end

fn builtins_cache
    render_docs("docs/builtins/cache", "Cache Functions", "builtins", "cache")
end

fn builtins_kv
    render_docs("docs/builtins/kv", "KV Store", "builtins", "kv")
end

fn builtins_solidb
    render_docs("docs/builtins/solidb", "Solidb", "builtins", "solidb")
end

fn builtins_rate_limit
    render_docs("docs/builtins/rate-limit", "Rate Limiting Functions", "builtins", "rate_limit")
end

fn builtins_security_headers
    render_docs("docs/builtins/security-headers", "Security Headers Functions", "builtins", "security_headers")
end

fn builtins_hardening
    render_docs("docs/builtins/hardening", "Server Hardening", "builtins", "hardening")
end

fn builtins_upload
    render_docs("docs/builtins/upload", "File Upload Functions", "builtins", "upload")
end

fn builtins_soap
    render_docs("docs/builtins/soap", "SOAP Class", "builtins", "soap")
end

fn builtins_pop3
    render_docs("docs/builtins/pop3", "POP3 Email Class", "builtins", "pop3")
end

fn builtins_mailer
    render_docs("docs/builtins/mailer", "Mailer", "builtins", "mailer")
end

fn builtins_markdown
    render_docs("docs/builtins/markdown", "Markdown Class", "builtins", "markdown")
end

fn builtins_image
    render_docs("docs/builtins/image", "Image Class", "builtins", "image")
end

fn builtins_pdf
    render_docs("docs/builtins/pdf", "PDF & Factur-X", "builtins", "pdf")
end

fn pdf_playground
    render_docs("docs/pdf_playground", "PDF Playground", "builtins", "pdf", true)
end

fn pdf_playground_render
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

fn builtins_file
    render_docs("docs/builtins/file", "File Class", "builtins", "file")
end

fn builtins_spreadsheet
    render_docs("docs/builtins/spreadsheet", "Spreadsheet Functions", "builtins", "spreadsheet")
end

# /docs/builtins/websocket merged into /docs/core-concepts/websockets
fn builtins_websocket
    redirect("/docs/core-concepts/websockets")
end

# ============================================================================
# Utility
# ============================================================================

fn utility_base64
    render_docs("docs/utility/base64", "Base64 Encoding", "utility", "base64")
end

fn utility_encoding
    render_docs("docs/utility/encoding", "Character Encodings", "utility", "encoding")
end

# ============================================================================ Testing
# ============================================================================

fn testing
    render_docs("docs/core-concepts/testing", "Testing", "testing", "testing")
end

fn testing_quick_reference
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

fn redirect_introduction
    redirect("/docs/getting-started/introduction")
end

fn redirect_installation
    redirect("/docs/getting-started/installation")
end

fn redirect_configuration
    redirect("/docs/getting-started/configuration")
end

fn redirect_routing
    redirect("/docs/core-concepts/routing")
end

fn redirect_controllers
    redirect("/docs/core-concepts/controllers")
end

fn redirect_middleware
    redirect("/docs/core-concepts/middleware")
end

fn redirect_views
    redirect("/docs/core-concepts/views")
end

fn redirect_websockets
    redirect("/docs/core-concepts/websockets")
end

fn redirect_liveview
    redirect("/docs/core-concepts/liveview")
end

fn redirect_i18n
    redirect("/docs/core-concepts/i18n")
end

fn redirect_request_params
    redirect("/docs/core-concepts/request-params")
end

fn redirect_error_pages
    redirect("/docs/core-concepts/error-pages")
end

fn redirect_database
    redirect("/docs/database/configuration")
end

fn redirect_models
    redirect("/docs/database/models")
end

fn redirect_migrations
    redirect("/docs/database/migrations")
end

fn redirect_authentication
    redirect("/docs/security/authentication")
end

fn redirect_sessions
    redirect("/docs/security/sessions")
end

fn redirect_validation
    redirect("/docs/builtins/validation")
end

fn redirect_live_reload
    redirect("/docs/development-tools/live-reload")
end

fn redirect_debugging
    redirect("/docs/development-tools/debugging")
end

fn redirect_scaffold
    redirect("/docs/development-tools/scaffold")
end

fn redirect_soli_language
    redirect("/docs/language")
end

# ============================================================================
# Core Concepts - State Machines
# ============================================================================

fn core_concepts_state_machines
    render_docs("docs/core-concepts/state-machines", "State Machines", "core_concepts", "state_machines")
end

# ============================================================================
# Blog
# ============================================================================

fn blog_index
    render_docs("docs/blog/index", "Blog", "blog", "index")
end

fn blog_google_oauth
    render_docs("docs/blog/google-oauth", "Implementing Google OAuth in SoliLang", "blog", "google_oauth")
end
