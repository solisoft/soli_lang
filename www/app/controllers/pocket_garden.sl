# The Pocket Garden that plays inside its blog post
# (docs/blog/pocket-garden-eui.md). The game is examples/pocket-garden, copied
# into pocket_garden/ beside this file by scripts/sync-pocket-garden.sh; this
# file brings its handler and view into the application for the
# `router_eui("pocket-garden", …)` line in config/routes.sl. No class:
# `pocket_garden#pocket_garden` names the global function once no controller
# of that name exists.
import "./pocket_garden/game.sl"
