# Eval fixture routes. Add resources in tasks, do not invent SDBQL.

get("/", "home#index", name: "root")
get("/health", "home#health")
