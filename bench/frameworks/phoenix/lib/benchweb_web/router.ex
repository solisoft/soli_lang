defmodule BenchWebWeb.Router do
  use BenchWebWeb, :router

  # Phoenix's default browser stack, unmodified — session, CSRF and secure
  # headers. It is the analogue of Django running its full default middleware
  # chain and Rails running ActionController, so the HTML rows stay matched.
  pipeline :browser do
    plug :accepts, ["html"]
    plug :fetch_session
    plug :fetch_live_flash
    plug :put_root_layout, html: {BenchWebWeb.Layouts, :root}
    plug :protect_from_forgery
    plug :put_secure_browser_headers
  end

  pipeline :api do
    plug :accepts, ["json"]
  end

  scope "/", BenchWebWeb do
    pipe_through :browser

    get "/template", BenchController, :template_only
    get "/db-template", BenchController, :db_template
  end

  # The write routes sit on :api for the same reason Django's are `@csrf_exempt`:
  # the benchmark client does not carry a CSRF token.
  scope "/", BenchWebWeb do
    pipe_through :api

    get "/json", BenchController, :json_only
    get "/db", BenchController, :db_json
    get "/db-hydrated", BenchController, :db_hydrated

    post "/w", BenchController, :w_create
    patch "/w", BenchController, :w_update
    delete "/w", BenchController, :w_delete
  end
end
