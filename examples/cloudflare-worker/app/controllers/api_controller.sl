class ApiController < Controller
  # GET /api/info — the facts the footer shows, as JSON
  def info(req)
    render_json(RuntimeInfo.of(req))
  end
end
