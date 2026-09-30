# Routes configuration
# This app is a benchmark harness for the Soli framework. The routes below
# exercise four distinct workloads so `oha` can measure each in isolation:
#   - json : pure JSON serialization (no DB, no view)
#   - view : server-side template rendering (no DB)
#   - db   : database reads
#   - crud : full create/read/update/delete write lifecycle

# Home + health
get("/", "home#index")
get("/health", "home#health")

# --- JSON serialization workload (no DB, no view) ---
get("/bench/json", "bench#json_one")
get("/bench/json_many", "bench#json_many")

# --- View rendering workload (no DB) ---
get("/bench/view", "bench#view")

# --- Realistic page: layout + partials + DB, complexity over 4 steps ---
get("/bench/page", "bench#page")

# --- DB read workload ---
get("/bench/db_one", "bench#db_one")
get("/bench/db_list", "bench#db_list")

# --- Seed helper (populate widgets before the db/crud runs) ---
get("/bench/seed", "bench#seed")

# --- Benchmark result ingestion (run_bench.sh POSTs each run here) ---
post("/bench/results", "bench#save_results")

# --- CRUD write lifecycle (JSON API) ---
post("/bench/crud", "bench#crud_create")
delete("/bench/crud", "bench#crud_delete_any")
get("/bench/crud/:id", "bench#crud_show")
put("/bench/crud/:id", "bench#crud_update")
delete("/bench/crud/:id", "bench#crud_delete")
