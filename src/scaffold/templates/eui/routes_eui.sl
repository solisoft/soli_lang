
# ------------------------------------------------------------------ EUI
# A native window, served by this application over a WebSocket rather
# than as HTML. Start the server, then point a client at the component:
#
#   soli serve . --port 5011
#   EUI_ALLOW_INSECURE_LOOPBACK=1 eui-client ws://127.0.0.1:5011/_eui/session/counter
#
# The environment variable is for loopback only; anywhere else the
# client requires wss and pins the manifest's publisher key.
#
# router_eui(component, handler, view):
#   handler — {event, params, state} -> state
#   view    — state -> node tree
router_eui("counter", "eui#counter", "eui#counter_view")
