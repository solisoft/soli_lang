class PagesController < Controller
  # GET / — what this is
  def index(req)
    @title = "Soli on Cloudflare Workers"
    @description = "A Soli web app inside a Cloudflare Worker: the interpreter compiled to WebAssembly, your " +
      "routes, controllers, views and models unchanged, on D1, Postgres through Hyperdrive, R2 and Queues."
    @runtime = RuntimeInfo.of(req)
  end

  # GET /guide — from an app to a deployed Worker
  def guide(req)
    @title = "Guide"
    @description = "From a Soli app to a deployed Cloudflare Worker in five steps: build the edge runtime, " +
      "soli new --edge, soli edge dev, wire D1, Postgres, R2, Images and Queues, deploy."
    @runtime = RuntimeInfo.of(req)
  end

  # GET /live — a LiveView whose socket is a Durable Object
  def live(req)
    @title = "LiveView"
    @description = "A Soli LiveView inside a Cloudflare Worker: the socket is held by a Durable Object that " +
      "hibernates between events, and every frame runs the same handler as soli serve."
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
