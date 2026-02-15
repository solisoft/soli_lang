// Documentation Controller
// Handles displaying documentation pages

fn index(req) {
    redirect("/docs/getting-started/introduction")
}

// Helper to render docs pages with consistent context and caching
fn render_docs(view, title, section, subsection) {
    let response = render(view, {
        "title": title,
        "section": section,
        "subsection": subsection,
        "layout": "layouts/docs"
    });
    // Add cache headers - browsers will cache for 1 hour, revalidate after
    response["headers"]["Cache-Control"] = "public, max-age=3600, stale-while-revalidate=86400";
    response
}

// ============================================================================
// Getting Started
// ============================================================================

fn getting_started_introduction(req) {
    render_docs("docs/getting-started/introduction", "Introduction", "getting_started", "introduction")
}

fn getting_started_installation(req) {
    render_docs("docs/getting-started/installation", "Installation", "getting_started", "installation")
}

// ============================================================================
// Core Concepts
// ============================================================================

fn core_concepts_routing(req) {
    render_docs("docs/core-concepts/routing", "Routing", "core_concepts", "routing")
}

fn core_concepts_controllers(req) {
    render_docs("docs/core-concepts/controllers", "Controllers", "core_concepts", "controllers")
}

fn core_concepts_middleware(req) {
    render_docs("docs/core-concepts/middleware", "Middleware", "core_concepts", "middleware")
}

fn core_concepts_views(req) {
    render_docs("docs/core-concepts/views", "Views", "core_concepts", "views")
}

fn core_concepts_websockets(req) {
    render_docs("docs/core-concepts/websockets", "WebSockets", "core_concepts", "websockets")
}

fn core_concepts_liveview(req) {
    render_docs("docs/core-concepts/liveview", "Live View", "core_concepts", "liveview")
}

fn core_concepts_i18n(req) {
    render_docs("docs/core-concepts/i18n", "Internationalization", "core_concepts", "i18n")
}

fn core_concepts_request_params(req) {
    render_docs("docs/core-concepts/request-params", "Request Parameters", "core_concepts", "request_params")
}

fn core_concepts_error_pages(req) {
    render_docs("docs/core-concepts/error-pages", "Error Pages", "core_concepts", "error_pages")
}

// ============================================================================
// Database
// ============================================================================

fn database_configuration(req) {
    render_docs("docs/database/configuration", "Database Configuration", "database", "configuration")
}

fn database_models(req) {
    render_docs("docs/database/models", "Models & ORM", "database", "models")
}

fn database_migrations(req) {
    render_docs("docs/database/migrations", "Migrations", "database", "migrations")
}

// ============================================================================
// Security
// ============================================================================

fn security_authentication(req) {
    render_docs("docs/security/authentication", "Authentication with JWT", "security", "authentication")
}

fn security_sessions(req) {
    render_docs("docs/security/sessions", "Session Management", "security", "sessions")
}

fn security_validation(req) {
    render_docs("docs/security/validation", "Input Validation", "security", "validation")
}

// ============================================================================
// Development Tools
// ============================================================================

fn development_tools_live_reload(req) {
    render_docs("docs/development-tools/live-reload", "Live Reload", "development_tools", "live_reload")
}

fn development_tools_debugging(req) {
    render_docs("docs/development-tools/debugging", "Debugging", "development_tools", "debugging")
}

fn development_tools_scaffold(req) {
    render_docs("docs/development-tools/scaffold", "Scaffold Generator", "development_tools", "scaffold")
}

// ============================================================================
// Language Reference
// ============================================================================

fn language_index(req) {
    render_docs("docs/language/index", "Soli Language Reference", "language", "index")
}

fn language_variables_types(req) {
    render_docs("docs/language/variables-types", "Variables & Types", "language", "variables_types")
}

fn language_operators(req) {
    render_docs("docs/language/operators", "Operators", "language", "operators")
}

fn language_control_flow(req) {
    render_docs("docs/language/control-flow", "Control Flow", "language", "control_flow")
}

fn language_functions(req) {
    render_docs("docs/language/functions", "Functions", "language", "functions")
}

fn language_strings(req) {
    render_docs("docs/language/strings", "Strings", "language", "strings")
}

fn language_arrays(req) {
    render_docs("docs/language/arrays", "Arrays", "language", "arrays")
}

fn language_hashes(req) {
    render_docs("docs/language/hashes", "Hashes", "language", "hashes")
}

fn language_collections(req) {
    render_docs("docs/language/collections", "Collections", "language", "collections")
}

fn language_classes_oop(req) {
    render_docs("docs/language/classes-oop", "Classes & OOP", "language", "classes_oop")
}

fn language_pattern_matching(req) {
    render_docs("docs/language/pattern-matching", "Pattern Matching", "language", "pattern_matching")
}

fn language_pipeline_operator(req) {
    render_docs("docs/language/pipeline-operator", "Pipeline Operator", "language", "pipeline_operator")
}

fn language_modules(req) {
    render_docs("docs/language/modules", "Modules", "language", "modules")
}

// ============================================================================
// Builtins Reference
// ============================================================================

fn builtins_index(req) {
    render_docs("docs/builtins/index", "Built-in Functions", "builtins", "index")
}

fn builtins_core(req) {
    render_docs("docs/builtins/core", "Core Functions", "builtins", "core")
}

fn builtins_system(req) {
    render_docs("docs/builtins/system", "System Functions", "builtins", "system")
}

fn builtins_http(req) {
    render_docs("docs/builtins/http", "HTTP Functions", "builtins", "http")
}

fn builtins_json(req) {
    render_docs("docs/builtins/json", "JSON Functions", "builtins", "json")
}

fn builtins_crypto(req) {
    render_docs("docs/builtins/crypto", "Crypto Functions", "builtins", "crypto")
}

fn builtins_jwt(req) {
    render_docs("docs/builtins/jwt", "JWT Functions", "builtins", "jwt")
}

fn builtins_regex(req) {
    render_docs("docs/builtins/regex", "Regex Functions", "builtins", "regex")
}

fn builtins_env(req) {
    render_docs("docs/builtins/env", "Environment Functions", "builtins", "env")
}

fn builtins_datetime(req) {
    render_docs("docs/builtins/datetime", "DateTime", "builtins", "datetime")
}

fn builtins_duration(req) {
    render_docs("docs/builtins/duration", "Duration", "builtins", "duration")
}

fn builtins_validation(req) {
    render_docs("docs/builtins/validation", "Validation Functions", "builtins", "validation")
}

fn builtins_session(req) {
    render_docs("docs/builtins/session", "Session Functions", "builtins", "session")
}

fn builtins_testing(req) {
    render_docs("docs/builtins/testing", "Testing Functions", "builtins", "testing")
}

fn builtins_i18n(req) {
    render_docs("docs/builtins/i18n", "I18n Functions", "builtins", "i18n")
}

fn builtins_cache(req) {
    render_docs("docs/builtins/cache", "Cache Functions", "builtins", "cache")
}

fn builtins_rate_limit(req) {
    render_docs("docs/builtins/rate-limit", "Rate Limiting Functions", "builtins", "rate_limit")
}

fn builtins_security_headers(req) {
    render_docs("docs/builtins/security-headers", "Security Headers Functions", "builtins", "security_headers")
}

fn builtins_upload(req) {
    render_docs("docs/builtins/upload", "File Upload Functions", "builtins", "upload")
}

fn builtins_soap(req) {
    render_docs("docs/builtins/soap", "SOAP Class", "builtins", "soap")
}

// ============================================================================
// Utility
// ============================================================================

fn utility_base64(req) {
    render_docs("docs/utility/base64", "Base64 Encoding", "utility", "base64")
}

// ============================================================================// Testing
// ============================================================================

fn testing(req) {
    render_docs("docs/core-concepts/testing", "Testing", "testing", "testing")
}

fn testing_quick_reference(req) {
    render_docs("docs/core-concepts/testing-quick-reference", "Testing Quick Reference", "testing", "testing_quick_reference")
}

// ============================================================================
// Backward Compatibility Redirects
// ============================================================================

fn redirect_introduction(req) {
    redirect("/docs/getting-started/introduction")
}

fn redirect_installation(req) {
    redirect("/docs/getting-started/installation")
}

fn redirect_routing(req) {
    redirect("/docs/core-concepts/routing")
}

fn redirect_controllers(req) {
    redirect("/docs/core-concepts/controllers")
}

fn redirect_middleware(req) {
    redirect("/docs/core-concepts/middleware")
}

fn redirect_views(req) {
    redirect("/docs/core-concepts/views")
}

fn redirect_websockets(req) {
    redirect("/docs/core-concepts/websockets")
}

fn redirect_liveview(req) {
    redirect("/docs/core-concepts/liveview")
}

fn redirect_i18n(req) {
    redirect("/docs/core-concepts/i18n")
}

fn redirect_request_params(req) {
    redirect("/docs/core-concepts/request-params")
}

fn redirect_error_pages(req) {
    redirect("/docs/core-concepts/error-pages")
}

fn redirect_database(req) {
    redirect("/docs/database/configuration")
}

fn redirect_models(req) {
    redirect("/docs/database/models")
}

fn redirect_migrations(req) {
    redirect("/docs/database/migrations")
}

fn redirect_authentication(req) {
    redirect("/docs/security/authentication")
}

fn redirect_sessions(req) {
    redirect("/docs/security/sessions")
}

fn redirect_validation(req) {
    redirect("/docs/security/validation")
}

fn redirect_live_reload(req) {
    redirect("/docs/development-tools/live-reload")
}

fn redirect_debugging(req) {
    redirect("/docs/development-tools/debugging")
}

fn redirect_scaffold(req) {
    redirect("/docs/development-tools/scaffold")
}

fn redirect_soli_language(req) {
    redirect("/docs/language")
}

// ============================================================================
// Core Concepts - State Machines
// ============================================================================

fn core_concepts_state_machines(req) {
    render_docs("docs/core-concepts/state-machines", "State Machines", "core_concepts", "state_machines")
}
