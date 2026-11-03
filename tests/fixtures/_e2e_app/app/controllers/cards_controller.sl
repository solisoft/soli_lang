# A page rendered from objects. The response cache cannot see what an
# object's methods read (a model's relations query the database), so it must
# render this page on every request.
class CardsController < Controller
  # GET /cards — one card no earlier request showed
  def index
    @cards = [Card.draw]
    render("cards/index", {}, {"layout": false})
  end
end
