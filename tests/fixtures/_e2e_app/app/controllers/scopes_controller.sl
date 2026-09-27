# Model scopes read in an action (VM path under `soli serve`).
#
# The VM had no scope lookup for a model class: `Gadget.owned` raised
# "Cannot access property", the server re-ran the action on the tree-walker,
# and only an action that had already written showed it, with a 500
# (2.6.3). The queries are built with `to_query`, so the fixture needs no
# database and the body shows each scope was applied, not just found.
class ScopesController < Controller
  def index
    bare = Gadget.owned.where({"size": 2}).to_query
    parens = Gadget.owned().to_query
    args = Gadget.of_kind("lamp").to_query
    render_text([Gadget.label(), bare, parens, args].join("\n"))
  end
end
