Rails.application.routes.draw do
  get "/health",   to: "health#show"
  get "/json",     to: "posts#json_only"
  get "/template", to: "posts#template_only"
  get "/db",       to: "posts#db_json"
  get "/db-template", to: "posts#db_template"
  get "/db-selectall", to: "posts#db_json_select_all"
  post   "/w", to: "posts#w_create"
  patch  "/w", to: "posts#w_update"
  delete "/w", to: "posts#w_delete"
end
