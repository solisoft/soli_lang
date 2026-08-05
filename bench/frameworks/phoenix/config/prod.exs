import Config

# `force_ssl` is REMOVED from the generated config, deliberately.
#
# Phoenix generates it with an exclude list for localhost, but any mismatch
# turns every benchmark request into a 301 — and `oha` counts a wall of 301s as
# 100% "success", which is the easiest way on this whole page to publish a
# meaningless number. The other stacks serve plain HTTP on loopback; so does
# this one.

# Phoenix logs one line per request through its telemetry handler at :info, so
# leaving :info here would make this the only stack paying for access logging —
# Django runs with `--access-logfile /dev/null`, FastAPI with `--no-access-log`.
config :logger, level: :warning

# Runtime production configuration, including reading
# of environment variables, is done on config/runtime.exs.
