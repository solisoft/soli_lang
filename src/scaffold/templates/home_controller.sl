# Home controller - handles the root routes

class HomeController < Controller
  # GET /
  def index
    @title = "Welcome"
    @description = "A web app built with Soli."
  end

  # GET /health
  def health
    {
      "status": 200,
      "headers": {"Content-Type": "application/json"},
      "body": "{\"status\":\"ok\"}"
    }
  end
end
