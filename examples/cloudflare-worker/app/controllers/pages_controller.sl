class PagesController < Controller
  # GET / — what this is
  def index(req)
    @title = "Soli on Cloudflare Workers"
    @runtime = RuntimeInfo.of(req)
  end

  # GET /guide — from an app to a deployed Worker
  def guide(req)
    @title = "Guide"
    @runtime = RuntimeInfo.of(req)
  end

  # GET /limits — what the edge build does and does not do
  def limits(req)
    @title = "Limits"
    @runtime = RuntimeInfo.of(req)
  end
end
