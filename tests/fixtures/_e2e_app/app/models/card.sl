# Not persisted. Each `Card.draw` has a title no earlier card had, so a page
# built from one changes on every request while the request stays the same:
# what a page of model records does after a record is edited.
class Card
  static drawn: Int = 0

  title: String

  new(title: String)
    @title = title
  end

  static def draw -> Card
    Card.drawn = Card.drawn + 1
    Card.new("card #{Card.drawn}")
  end
end
