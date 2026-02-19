# ============================================================================
# Solilang MVC Routes Configuration
# ============================================================================

# Note: Middleware in app/middleware/ is loaded automatically.
# - global_only middleware runs for ALL requests
# - scope_only middleware must be explicitly scoped using: middleware("name", -> { ... })

# Root path
get("/", "home#index");

# Public routes
get("/health", "home#health");
get("/up", "home#up");

# ============================================================================
# Documentation
# ============================================================================

get("/docs", "docs#index");

# Getting Started
get("/docs/getting-started/introduction", "docs#getting_started_introduction");
get("/docs/getting-started/installation", "docs#getting_started_installation");

# Core Concepts
get("/docs/core-concepts/routing", "docs#core_concepts_routing");
get("/docs/core-concepts/controllers", "docs#core_concepts_controllers");
get("/docs/core-concepts/middleware", "docs#core_concepts_middleware");
get("/docs/core-concepts/views", "docs#core_concepts_views");
get("/docs/core-concepts/websockets", "docs#core_concepts_websockets");
get("/docs/core-concepts/liveview", "docs#core_concepts_liveview");
get("/docs/core-concepts/i18n", "docs#core_concepts_i18n");
get("/docs/core-concepts/request-params", "docs#core_concepts_request_params");
get("/docs/core-concepts/error-pages", "docs#core_concepts_error_pages");

# Database
get("/docs/database/configuration", "docs#database_configuration");
get("/docs/database/models", "docs#database_models");
get("/docs/database/migrations", "docs#database_migrations");

# Security
get("/docs/security/authentication", "docs#security_authentication");
get("/docs/security/sessions", "docs#security_sessions");
get("/docs/security/validation", "docs#security_validation");

# Development Tools
get("/docs/development-tools/live-reload", "docs#development_tools_live_reload");
get("/docs/development-tools/debugging", "docs#development_tools_debugging");
get("/docs/development-tools/scaffold", "docs#development_tools_scaffold");

# Language Reference
get("/docs/language", "docs#language_index");
get("/docs/language/variables-types", "docs#language_variables_types");
get("/docs/language/operators", "docs#language_operators");
get("/docs/language/control-flow", "docs#language_control_flow");
get("/docs/language/error-handling", "docs#language_error_handling");
get("/docs/language/functions", "docs#language_functions");
get("/docs/language/strings", "docs#language_strings");
get("/docs/language/arrays", "docs#language_arrays");
get("/docs/language/hashes", "docs#language_hashes");
get("/docs/language/collections", "docs#language_collections");
get("/docs/language/classes-oop", "docs#language_classes_oop");
get("/docs/language/pattern-matching", "docs#language_pattern_matching");
get("/docs/language/pipeline-operator", "docs#language_pipeline_operator");
get("/docs/language/decimal", "docs#language_decimal");
get("/docs/language/modules", "docs#language_modules");
get("/docs/language/blocks", "docs#language_blocks");
get("/docs/language/linting", "docs#language_linting");

# Builtins Reference
get("/docs/builtins", "docs#builtins_index");
get("/docs/builtins/core", "docs#builtins_core");
get("/docs/builtins/system", "docs#builtins_system");
get("/docs/builtins/http", "docs#builtins_http");
get("/docs/builtins/json", "docs#builtins_json");
get("/docs/builtins/crypto", "docs#builtins_crypto");
get("/docs/builtins/jwt", "docs#builtins_jwt");
get("/docs/builtins/regex", "docs#builtins_regex");
get("/docs/builtins/env", "docs#builtins_env");
get("/docs/builtins/datetime", "docs#builtins_datetime");
get("/docs/builtins/duration", "docs#builtins_duration");
get("/docs/builtins/validation", "docs#builtins_validation");
get("/docs/builtins/session", "docs#builtins_session");
get("/docs/builtins/testing", "docs#builtins_testing");
get("/docs/builtins/i18n", "docs#builtins_i18n");
get("/docs/builtins/cache", "docs#builtins_cache");
get("/docs/builtins/rate-limit", "docs#builtins_rate_limit");
get("/docs/builtins/security-headers", "docs#builtins_security_headers");
get("/docs/builtins/upload", "docs#builtins_upload");
get("/docs/builtins/soap", "docs#builtins_soap");

# Utility
get("/docs/utility/base64", "docs#utility_base64");

# Testing
get("/docs/testing", "docs#testing");
get("/docs/testing-quick-reference", "docs#testing_quick_reference");

# ============================================================================
# Backward Compatibility Redirects (old flat URLs -> new hierarchical URLs)
# ============================================================================

get("/docs/introduction", "docs#redirect_introduction");
get("/docs/installation", "docs#redirect_installation");
get("/docs/routing", "docs#redirect_routing");
get("/docs/controllers", "docs#redirect_controllers");
get("/docs/middleware", "docs#redirect_middleware");
get("/docs/views", "docs#redirect_views");
get("/docs/websockets", "docs#redirect_websockets");
get("/docs/liveview", "docs#redirect_liveview");
get("/docs/i18n", "docs#redirect_i18n");
get("/docs/request-params", "docs#redirect_request_params");
get("/docs/error-pages", "docs#redirect_error_pages");
get("/docs/database", "docs#redirect_database");
get("/docs/models", "docs#redirect_models");
get("/docs/migrations", "docs#redirect_migrations");
get("/docs/authentication", "docs#redirect_authentication");
get("/docs/sessions", "docs#redirect_sessions");
get("/docs/validation", "docs#redirect_validation");
get("/docs/live-reload", "docs#redirect_live_reload");
get("/docs/debugging", "docs#redirect_debugging");
get("/docs/scaffold", "docs#redirect_scaffold");
get("/docs/soli-language", "docs#redirect_soli_language");

# ============================================================================
# WebSocket Demo
# ============================================================================

get("/websocket", "websocket#demo");
router_websocket("/ws/chat", "websocket#chat_handler");

# ============================================================================
# LiveView Routes
# ============================================================================

# Register LiveView components with their controller handlers
router_live("counter", "live#counter");
router_live("metrics", "live#metrics");

# ============================================================================
# Users Controller - Authentication, Sessions, and Validation Demo
# ============================================================================

# Authentication routes
get("/users/login", "users#login");
post("/users/login", "users#login_post");
get("/users/register", "users#register");
post("/users/register", "users#register_post");
get("/users/logout", "users#logout");
get("/users/profile", "users#profile");

# Session management
get("/users/regenerate-session", "users#regenerate_session");

# Validation demo
get("/users/validation-demo", "users#validation_demo");
post("/users/validate-registration", "users#validate_registration");

# JWT demo endpoints
post("/users/create-token", "users#create_token");
post("/users/verify-token", "users#verify_token");
post("/users/decode-token", "users#decode_token");

# ============================================================================
# State Machine Controller - REST API for State Machine management
# ============================================================================

# API endpoints
get("/api/state-machines", "state_machines#list");
post("/api/state-machines", "state_machines#create");
get("/api/state-machines/:id", "state_machines#get");
delete("/api/state-machines/:id", "state_machines#delete");
post("/api/state-machines/:id/transition", "state_machines#transition");
put("/api/state-machines/:id/context", "state_machines#set_context");
get("/api/state-machines/:id/context/:key", "state_machines#get_context");

# Demo page
get("/state-machines", "state_machines#demo");

# ============================================================================
# Documentation
# ============================================================================

get("/docs/core-concepts/state-machines", "docs#core_concepts_state_machines");

# ============================================================================
# Wildcard Route Examples
# ============================================================================
//
# These demonstrate the dynamic action resolution feature:
# Path /wildcard/* maps to wildcard_controller.sl functions
//
# Example: /wildcard/demo → wildcard#demo

get("/wildcard/*", "wildcard#*");

# ============================================================================
# Splat Route Examples
# ============================================================================
//
# Splat routes use *param_name to capture remaining path segments
# The captured value includes a leading slash

get("/files/*filepath", "home#files_demo");
get("/api/*version/users/*id", "home#api_demo");
get("/*catchall", "home#catchall_demo");
