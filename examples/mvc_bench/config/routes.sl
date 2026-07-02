get("/bench/health", "bench#health");
get("/bench/hello", "bench#hello");
// Engine-demotion regression probes (see soli_vm_handler_demotions_total).
get("/bench/named", "bench#named");
get("/bench/zero", "probe#zero");
get("/bench/compute", "bench#compute");
get("/bench/compute_named", "bench#compute_named");
