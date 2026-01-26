// Documentation Controller
// Handles displaying documentation pages

fn index(req: Any) -> Any {
    return redirect("/docs/getting-started/introduction");
}

// Helper to load navigation structure
fn get_docs_structure() -> Any {
    let content = slurp("config/docs_structure.json");
    if (content == null) {
        return [];
    }
    let parsed = json_parse(content);
    if (parsed == null) {
        return [];
    }
    return parsed;
}

// Helper to render docs pages with consistent context
fn render_docs(view: String, title: String, section: String, subsection: String) -> Any {
    return render(view, {
        "title": title,
        "section": section,
        "subsection": subsection,
        "layout": "layouts/docs",
        "docs_structure": get_docs_structure()
    });
}

// ============================================================================
// Getting Started
// ============================================================================

fn getting_started_introduction(req: Any) -> Any {
    return render_docs("docs/getting-started/introduction", "Introduction", "getting_started", "introduction");
}

fn getting_started_installation(req: Any) -> Any {
    return render_docs("docs/getting-started/installation", "Installation", "getting_started", "installation");
}

// ============================================================================
// Core Concepts
// ============================================================================

fn core_concepts_routing(req: Any) -> Any {
    return render_docs("docs/core-concepts/routing", "Routing", "core_concepts", "routing");
}

fn core_concepts_controllers(req: Any) -> Any {
    return render_docs("docs/core-concepts/controllers", "Controllers", "core_concepts", "controllers");
}

fn core_concepts_middleware(req: Any) -> Any {
    return render_docs("docs/core-concepts/middleware", "Middleware", "core_concepts", "middleware");
}

fn core_concepts_views(req: Any) -> Any {
    return render_docs("docs/core-concepts/views", "Views", "core_concepts", "views");
}

fn core_concepts_websockets(req: Any) -> Any {
    return render_docs("docs/core-concepts/websockets", "WebSockets", "core_concepts", "websockets");
}

fn core_concepts_liveview(req: Any) -> Any {
    return render_docs("docs/core-concepts/liveview", "Live View", "core_concepts", "liveview");
}

fn core_concepts_i18n(req: Any) -> Any {
    return render_docs("docs/core-concepts/i18n", "Internationalization", "core_concepts", "i18n");
}

fn core_concepts_request_params(req: Any) -> Any {
    return render_docs("docs/core-concepts/request-params", "Request Parameters", "core_concepts", "request_params");
}

fn core_concepts_error_pages(req: Any) -> Any {
    return render_docs("docs/core-concepts/error-pages", "Error Pages", "core_concepts", "error_pages");
}

// ============================================================================
// Database
// ============================================================================

fn database_configuration(req: Any) -> Any {
    return render_docs("docs/database/configuration", "Database Configuration", "database", "configuration");
}

fn database_models(req: Any) -> Any {
    return render_docs("docs/database/models", "Models & ORM", "database", "models");
}

fn database_migrations(req: Any) -> Any {
    return render_docs("docs/database/migrations", "Migrations", "database", "migrations");
}

// ============================================================================
// Security
// ============================================================================

fn security_authentication(req: Any) -> Any {
    return render_docs("docs/security/authentication", "Authentication with JWT", "security", "authentication");
}

fn security_sessions(req: Any) -> Any {
    return render_docs("docs/security/sessions", "Session Management", "security", "sessions");
}

fn security_validation(req: Any) -> Any {
    return render_docs("docs/security/validation", "Input Validation", "security", "validation");
}

// ============================================================================
// Development Tools
// ============================================================================

fn development_tools_live_reload(req: Any) -> Any {
    return render_docs("docs/development-tools/live-reload", "Live Reload", "development_tools", "live_reload");
}

fn development_tools_debugging(req: Any) -> Any {
    return render_docs("docs/development-tools/debugging", "Debugging", "development_tools", "debugging");
}

fn development_tools_scaffold(req: Any) -> Any {
    return render_docs("docs/development-tools/scaffold", "Scaffold Generator", "development_tools", "scaffold");
}

// ============================================================================
// Language Reference
// ============================================================================

fn language_index(req: Any) -> Any {
    return render_docs("docs/language/index", "Soli Language Reference", "language", "index");
}

fn language_variables_types(req: Any) -> Any {
    return render_docs("docs/language/variables-types", "Variables & Types", "language", "variables_types");
}

fn language_operators(req: Any) -> Any {
    return render_docs("docs/language/operators", "Operators", "language", "operators");
}

fn language_control_flow(req: Any) -> Any {
    return render_docs("docs/language/control-flow", "Control Flow", "language", "control_flow");
}

fn language_functions(req: Any) -> Any {
    return render_docs("docs/language/functions", "Functions", "language", "functions");
}

fn language_collections(req: Any) -> Any {
    return render_docs("docs/language/collections", "Collections", "language", "collections");
}

fn language_classes_oop(req: Any) -> Any {
    return render_docs("docs/language/classes-oop", "Classes & OOP", "language", "classes_oop");
}

fn language_pattern_matching(req: Any) -> Any {
    return render_docs("docs/language/pattern-matching", "Pattern Matching", "language", "pattern_matching");
}

fn language_pipeline_operator(req: Any) -> Any {
    return render_docs("docs/language/pipeline-operator", "Pipeline Operator", "language", "pipeline_operator");
}

fn language_modules(req: Any) -> Any {
    return render_docs("docs/language/modules", "Modules", "language", "modules");
}

// ============================================================================
// Builtins Reference
// ============================================================================

fn builtins_index(req: Any) -> Any {
    return render_docs("docs/builtins/index", "Built-in Functions", "builtins", "index");
}

fn builtins_core(req: Any) -> Any {
    return render_docs("docs/builtins/core", "Core Functions", "builtins", "core");
}

fn builtins_http(req: Any) -> Any {
    return render_docs("docs/builtins/http", "HTTP Functions", "builtins", "http");
}

fn builtins_json(req: Any) -> Any {
    return render_docs("docs/builtins/json", "JSON Functions", "builtins", "json");
}

fn builtins_crypto(req: Any) -> Any {
    return render_docs("docs/builtins/crypto", "Crypto Functions", "builtins", "crypto");
}

fn builtins_jwt(req: Any) -> Any {
    return render_docs("docs/builtins/jwt", "JWT Functions", "builtins", "jwt");
}

fn builtins_regex(req: Any) -> Any {
    return render_docs("docs/builtins/regex", "Regex Functions", "builtins", "regex");
}

fn builtins_env(req: Any) -> Any {
    return render_docs("docs/builtins/env", "Environment Functions", "builtins", "env");
}

fn builtins_datetime(req: Any) -> Any {
    return render_docs("docs/builtins/datetime", "DateTime", "builtins", "datetime");
}

fn builtins_duration(req: Any) -> Any {
    return render_docs("docs/builtins/duration", "Duration", "builtins", "duration");
}

fn builtins_validation(req: Any) -> Any {
    return render_docs("docs/builtins/validation", "Validation Functions", "builtins", "validation");
}

fn builtins_session(req: Any) -> Any {
    return render_docs("docs/builtins/session", "Session Functions", "builtins", "session");
}

fn builtins_testing(req: Any) -> Any {
    return render_docs("docs/builtins/testing", "Testing Functions", "builtins", "testing");
}

fn builtins_i18n(req: Any) -> Any {
    return render_docs("docs/builtins/i18n", "I18n Functions", "builtins", "i18n");
}

// ============================================================================
// Testing
// ============================================================================

fn testing(req: Any) -> Any {
    return render_docs("docs/core-concepts/testing", "Testing", "testing", "testing");
}

fn testing_quick_reference(req: Any) -> Any {
    return render_docs("docs/core-concepts/testing-quick-reference", "Testing Quick Reference", "testing", "testing_quick_reference");
}

// ============================================================================
// Backward Compatibility Redirects
// ============================================================================

fn redirect_introduction(req: Any) -> Any {
    return redirect("/docs/getting-started/introduction");
}

fn redirect_installation(req: Any) -> Any {
    return redirect("/docs/getting-started/installation");
}

fn redirect_routing(req: Any) -> Any {
    return redirect("/docs/core-concepts/routing");
}

fn redirect_controllers(req: Any) -> Any {
    return redirect("/docs/core-concepts/controllers");
}

fn redirect_middleware(req: Any) -> Any {
    return redirect("/docs/core-concepts/middleware");
}

fn redirect_views(req: Any) -> Any {
    return redirect("/docs/core-concepts/views");
}

fn redirect_websockets(req: Any) -> Any {
    return redirect("/docs/core-concepts/websockets");
}

fn redirect_liveview(req: Any) -> Any {
    return redirect("/docs/core-concepts/liveview");
}

fn redirect_i18n(req: Any) -> Any {
    return redirect("/docs/core-concepts/i18n");
}

fn redirect_request_params(req: Any) -> Any {
    return redirect("/docs/core-concepts/request-params");
}

fn redirect_error_pages(req: Any) -> Any {
    return redirect("/docs/core-concepts/error-pages");
}

fn redirect_database(req: Any) -> Any {
    return redirect("/docs/database/configuration");
}

fn redirect_models(req: Any) -> Any {
    return redirect("/docs/database/models");
}

fn redirect_migrations(req: Any) -> Any {
    return redirect("/docs/database/migrations");
}

fn redirect_authentication(req: Any) -> Any {
    return redirect("/docs/security/authentication");
}

fn redirect_sessions(req: Any) -> Any {
    return redirect("/docs/security/sessions");
}

fn redirect_validation(req: Any) -> Any {
    return redirect("/docs/security/validation");
}

fn redirect_live_reload(req: Any) -> Any {
    return redirect("/docs/development-tools/live-reload");
}

fn redirect_debugging(req: Any) -> Any {
    return redirect("/docs/development-tools/debugging");
}

fn redirect_scaffold(req: Any) -> Any {
    return redirect("/docs/development-tools/scaffold");
}

fn redirect_soli_language(req: Any) -> Any {
    return redirect("/docs/language");
}
