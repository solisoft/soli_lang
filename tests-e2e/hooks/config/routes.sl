get("/", "hooks_test#index");
get("/locked", "hooks_test#locked");
get("/empty_hook", "hooks_test#empty_hook");
get("/halt_in_action", "hooks_test#halt_in_action");
get("/render_with_data", "hooks_test#render_with_data");
get("/redirect_elsewhere", "hooks_test#redirect_elsewhere");
get("/after_redirect", "hooks_test#after_redirect");
get("/after_marked", "hooks_test#after_marked");
get("/param_shadow", "hooks_test#param_shadow");
get("/render_with_hash_arg", "hooks_test#render_with_hash_arg");

# Coverage-expansion routes: JSON APIs, sessions, form POST, error paths.
post("/api/echo", "api_test#echo_json");
get("/api/thing", "api_test#thing");
post("/api/form_echo", "api_test#form_echo");
post("/api/login", "api_test#login");
get("/api/me", "api_test#me");
post("/api/logout", "api_test#logout");
get("/api/boom", "api_test#boom");
get("/api/middleware_stamp", "api_test#echo_middleware_stamp");
