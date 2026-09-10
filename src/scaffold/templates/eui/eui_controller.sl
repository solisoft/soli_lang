# An EUI component: two functions and a route.
#
#   config/routes.sl
#   router_eui("counter", "eui#counter", "eui#counter_view")
#            component name ─┘   handler ─┘         view ─┘
#
# The handler takes an event and the state this instance last returned,
# and gives the next state. The view takes that state and gives a node
# tree. Neither holds a connection: the runtime keeps the tree it sent,
# diffs the new one against it, and puts the difference on the wire.
#
# Everything the view builds comes from `eui_builders.sl` beside this
# file — the reference catalogue, spec/03-widgets.md §4. It is plain
# Soli: read it, change it, add to it. Nothing there is native.
#
# That one file arrives as it is written upstream, which is not what
# `soli fmt` would write: it holds its style tables one row to a line,
# and the formatter would give each key a line of its own. `soli fmt`
# will offer to rewrite it. Until you have made the catalogue yours,
# there is nothing to gain by letting it — `soli lint` passes either
# way, and a formatted copy is a 3 000-line difference from the file
# any fix upstream will be written against.

def counter(event_data)
  event = event_data["event"]
  state = event_data["state"]
  count = state["count"] ?? 0

  # `connect` and `viewport` arrive too; anything unnamed keeps the state
  # as it was, which is what the last branch does.
  if event == "increment"
    {"count": count + 1}
  elsif event == "decrement"
    {"count": count - 1}
  else
    {"count": count}
  end
end

def counter_view(state)
  count = state["count"] ?? 0

  column(
    {"pad": 6, "gap": 4, "align": "start", "bg": "surface.base", "height": "100%"},
    [
      h1("Counter"),
      card({"gap": 3, "align": "start"}, [
        # Keyed, so the diff matches this node across renders and sends
        # the number rather than the box around it.
        keyed("value", text(count.to_s, {"size": 7, "weight": "bold"})),
        row({"gap": 2}, [
          secondary_button("−", "decrement"),
          button("+", "increment")
        ])
      ]),
      muted("Each click is a round trip; what comes back is the difference.")
    ]
  )
end
