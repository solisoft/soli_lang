// Documentation Controller
// Handles displaying documentation pages

fn index(req)
    redirect("/docs/getting-started/introduction")
end

// Helper to render docs pages with consistent context and caching
fn render_docs(view, title, section, subsection)
    let response = render(view, {
        "title": title,
        "section": section,
        "subsection": subsection,
        "layout": "layouts/docs"
    })
    // Add cache headers - browsers will cache for 1 hour, revalidate after
    response["headers"]["Cache-Control"] = "public, max-age=3600, stale-while-revalidate=86400"
    response
end

// ============================================================================
// Getting Started
// ============================================================================

fn getting_started_introduction(req)
    render_docs("docs/getting-started/introduction", "Introduction", "getting_started", "introduction")
end

fn getting_started_installation(req)
    render_docs("docs/getting-started/installation", "Installation", "getting_started", "installation")
end

// ============================================================================
// Core Concepts
// ============================================================================

fn core_concepts_routing(req)
    render_docs("docs/core-concepts/routing", "Routing", "core_concepts", "routing")
end

fn core_concepts_controllers(req)
    render_docs("docs/core-concepts/controllers", "Controllers", "core_concepts", "controllers")
end

fn core_concepts_middleware(req)
    render_docs("docs/core-concepts/middleware", "Middleware", "core_concepts", "middleware")
end

fn core_concepts_views(req)
    render_docs("docs/core-concepts/views", "Views", "core_concepts", "views")
end

fn core_concepts_websockets(req)
    render_docs("docs/core-concepts/websockets", "WebSockets", "core_concepts", "websockets")
end

fn core_concepts_liveview(req)
    render_docs("docs/core-concepts/liveview", "Live View", "core_concepts", "liveview")
end

fn core_concepts_i18n(req)
    render_docs("docs/core-concepts/i18n", "Internationalization", "core_concepts", "i18n")
end

fn core_concepts_request_params(req)
    render_docs("docs/core-concepts/request-params", "Request Parameters", "core_concepts", "request_params")
end

fn core_concepts_error_pages(req)
    render_docs("docs/core-concepts/error-pages", "Error Pages", "core_concepts", "error_pages")
end

// ============================================================================
// Database
// ============================================================================

fn database_configuration(req)
    render_docs("docs/database/configuration", "Database Configuration", "database", "configuration")
end

fn database_models(req)
    render_docs("docs/database/models", "Models & ORM", "database", "models")
end

fn database_migrations(req)
    render_docs("docs/database/migrations", "Migrations", "database", "migrations")
end

// ============================================================================
// Security
// ============================================================================

fn security_authentication(req)
    render_docs("docs/security/authentication", "Authentication with JWT", "security", "authentication")
end

fn security_sessions(req)
    render_docs("docs/security/sessions", "Session Management", "security", "sessions")
end

fn security_validation(req)
    render_docs("docs/security/validation", "Input Validation", "security", "validation")
end

// ============================================================================
// Development Tools
// ============================================================================

fn development_tools_live_reload(req)
    render_docs("docs/development-tools/live-reload", "Live Reload", "development_tools", "live_reload")
end

fn development_tools_debugging(req)
    render_docs("docs/development-tools/debugging", "Debugging", "development_tools", "debugging")
end

fn development_tools_scaffold(req)
    render_docs("docs/development-tools/scaffold", "Scaffold Generator", "development_tools", "scaffold")
end

// ============================================================================
// Language Reference
// ============================================================================

fn language_index(req)
    render_docs("docs/language/index", "Soli Language Reference", "language", "index")
end

fn language_variables_types(req)
    render_docs("docs/language/variables-types", "Variables & Types", "language", "variables_types")
end

fn language_operators(req)
    render_docs("docs/language/operators", "Operators", "language", "operators")
end

fn language_control_flow(req)
    render_docs("docs/language/control-flow", "Control Flow", "language", "control_flow")
end

fn language_error_handling(req)
    render_docs("docs/language/error-handling", "Error Handling", "language", "error_handling")
end

fn language_functions(req)
    render_docs("docs/language/functions", "Functions", "language", "functions")
end

fn language_strings(req)
    render_docs("docs/language/strings", "Strings", "language", "strings")
end

fn language_arrays(req)
    render_docs("docs/language/arrays", "Arrays", "language", "arrays")
end

fn language_hashes(req)
    render_docs("docs/language/hashes", "Hashes", "language", "hashes")
end

fn language_collections(req)
    render_docs("docs/language/collections", "Collections", "language", "collections")
end

fn language_classes_oop(req)
    render_docs("docs/language/classes-oop", "Classes & OOP", "language", "classes_oop")
end

fn language_pattern_matching(req)
    render_docs("docs/language/pattern-matching", "Pattern Matching", "language", "pattern_matching")
end

fn language_pipeline_operator(req)
    render_docs("docs/language/pipeline-operator", "Pipeline Operator", "language", "pipeline_operator")
end

fn language_modules(req)
    render_docs("docs/language/modules", "Modules", "language", "modules")
end

fn language_decimal(req)
    render_docs("docs/language/decimal", "Decimal", "language", "decimal")
end

fn language_blocks(req)
    render_docs("docs/language/blocks", "Block Syntax", "language", "blocks")
end

fn language_linting(req)
    render_docs("docs/language/linting", "Linting", "language", "linting")
end

// ============================================================================
// Builtins Reference
// ============================================================================

fn builtins_index(req)
    render_docs("docs/builtins/index", "Built-in Functions", "builtins", "index")
end

fn builtins_core(req)
    render_docs("docs/builtins/core", "Core Functions", "builtins", "core")
end

fn builtins_system(req)
    render_docs("docs/builtins/system", "System Functions", "builtins", "system")
end

fn builtins_http(req)
    render_docs("docs/builtins/http", "HTTP Functions", "builtins", "http")
end

fn builtins_json(req)
    render_docs("docs/builtins/json", "JSON Functions", "builtins", "json")
end

fn builtins_crypto(req)
    render_docs("docs/builtins/crypto", "Crypto Functions", "builtins", "crypto")
end

fn builtins_jwt(req)
    render_docs("docs/builtins/jwt", "JWT Functions", "builtins", "jwt")
end

fn builtins_regex(req)
    render_docs("docs/builtins/regex", "Regex Functions", "builtins", "regex")
end

fn builtins_env(req)
    render_docs("docs/builtins/env", "Environment Functions", "builtins", "env")
end

fn builtins_datetime(req)
    render_docs("docs/builtins/datetime", "DateTime", "builtins", "datetime")
end

fn builtins_duration(req)
    render_docs("docs/builtins/duration", "Duration", "builtins", "duration")
end

fn builtins_validation(req)
    render_docs("docs/builtins/validation", "Validation Functions", "builtins", "validation")
end

fn builtins_session(req)
    render_docs("docs/builtins/session", "Session Functions", "builtins", "session")
end

fn builtins_testing(req)
    render_docs("docs/builtins/testing", "Testing Functions", "builtins", "testing")
end

fn builtins_i18n(req)
    render_docs("docs/builtins/i18n", "I18n Functions", "builtins", "i18n")
end

fn builtins_cache(req)
    render_docs("docs/builtins/cache", "Cache Functions", "builtins", "cache")
end

fn builtins_rate_limit(req)
    render_docs("docs/builtins/rate-limit", "Rate Limiting Functions", "builtins", "rate_limit")
end

fn builtins_security_headers(req)
    render_docs("docs/builtins/security-headers", "Security Headers Functions", "builtins", "security_headers")
end

fn builtins_upload(req)
    render_docs("docs/builtins/upload", "File Upload Functions", "builtins", "upload")
end

fn builtins_soap(req)
    render_docs("docs/builtins/soap", "SOAP Class", "builtins", "soap")
end

// ============================================================================
// Utility
// ============================================================================

fn utility_base64(req)
    render_docs("docs/utility/base64", "Base64 Encoding", "utility", "base64")
end

// ============================================================================// Testing
// ============================================================================

fn testing(req)
    render_docs("docs/core-concepts/testing", "Testing", "testing", "testing")
end

fn testing_quick_reference(req)
    render_docs("docs/core-concepts/testing-quick-reference", "Testing Quick Reference", "testing", "testing_quick_reference")
end

// ============================================================================
// Backward Compatibility Redirects
// ============================================================================

fn redirect_introduction(req)
    redirect("/docs/getting-started/introduction")
end

fn redirect_installation(req)
    redirect("/docs/getting-started/installation")
end

fn redirect_routing(req)
    redirect("/docs/core-concepts/routing")
end

fn redirect_controllers(req)
    redirect("/docs/core-concepts/controllers")
end

fn redirect_middleware(req)
    redirect("/docs/core-concepts/middleware")
end

fn redirect_views(req)
    redirect("/docs/core-concepts/views")
end

fn redirect_websockets(req)
    redirect("/docs/core-concepts/websockets")
end

fn redirect_liveview(req)
    redirect("/docs/core-concepts/liveview")
end

fn redirect_i18n(req)
    redirect("/docs/core-concepts/i18n")
end

fn redirect_request_params(req)
    redirect("/docs/core-concepts/request-params")
end

fn redirect_error_pages(req)
    redirect("/docs/core-concepts/error-pages")
end

fn redirect_database(req)
    redirect("/docs/database/configuration")
end

fn redirect_models(req)
    redirect("/docs/database/models")
end

fn redirect_migrations(req)
    redirect("/docs/database/migrations")
end

fn redirect_authentication(req)
    redirect("/docs/security/authentication")
end

fn redirect_sessions(req)
    redirect("/docs/security/sessions")
end

fn redirect_validation(req)
    redirect("/docs/security/validation")
end

fn redirect_live_reload(req)
    redirect("/docs/development-tools/live-reload")
end

fn redirect_debugging(req)
    redirect("/docs/development-tools/debugging")
end

fn redirect_scaffold(req)
    redirect("/docs/development-tools/scaffold")
end

fn redirect_soli_language(req)
    redirect("/docs/language")
end

// ============================================================================
// Core Concepts - State Machines
// ============================================================================

fn core_concepts_state_machines(req)
    render_docs("docs/core-concepts/state-machines", "State Machines", "core_concepts", "state_machines")
end
