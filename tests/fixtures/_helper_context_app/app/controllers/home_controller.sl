# The view helpers' `req` must be this request's on every page, including
# those rendered before or without the action: a route miss's custom 404
# and a view a middleware renders.
class HomeController < Controller
  # GET /ping
  def ping(req)
    render_json({"pong": true})
  end

  # GET /me
  # GET /blocked — answered by the gate middleware
  def me(req)
    @seen = 1
  end
end
