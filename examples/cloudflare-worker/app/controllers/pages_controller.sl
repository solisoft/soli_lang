class PagesController < Controller
  # GET / — what this is
  def index(req)
    @title = "Soli on Cloudflare Workers"
    @description = "A Soli web app inside a Cloudflare Worker: the interpreter compiled to WebAssembly, your " +
      "routes, controllers, views and models unchanged, and the data in Cloudflare D1."
    @runtime = RuntimeInfo.of(req)
  end

  # GET /guide — from an app to a deployed Worker
  def guide(req)
    @title = "Guide"
    @description = "From a Soli app folder to a deployed Cloudflare Worker in five steps: build the edge runtime, " +
      "package the app, run it on workerd, configure it, deploy."
    @runtime = RuntimeInfo.of(req)
  end

  # GET /limits — what the edge build does and does not do
  def limits(req)
    @title = "Limits"
    @description = "What a Soli app runs inside a Cloudflare Worker, and what stays with soli serve: databases, " +
      "sessions, streaming, jobs, size."
    @runtime = RuntimeInfo.of(req)
  end
end
