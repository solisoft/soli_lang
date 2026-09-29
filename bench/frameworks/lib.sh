# lib.sh — sourced by the harness scripts. Knobs for running the suite on a box
# other than the one the published session came from. All default to the
# original behaviour: nothing pinned, SoliDB on 6745.
#
#   SDB_PORT      the SoliDB the Soli stack reads and the sweep charges CPU to
#   SERVER_CPUS   cpu list (taskset -c syntax) every stack is started on
#   CLIENT_CPUS   cpu list oha runs on
#
# Pinning is what makes a 32-thread box give every stack the same 16 threads
# while the load generator and the databases run on cores of their own — with
# all three sharing every core, a faster server also steals more of the
# client's time, and the comparison measures the scheduler as much as the stack.

SDB_PORT="${SDB_PORT:-6745}"
# Rows in the write table. The stacks read the same variable at boot, so set it
# before start.sh. 800,000 was sized for the original box; a box fast enough to
# exhaust it inside a 30s delete cell turns that cell into a measure of misses.
WPOOL="${WPOOL:-800000}"
export WPOOL
SDB="${SDB:-http://localhost:$SDB_PORT/_api/database/default}"

# oha, on CLIENT_CPUS when set. A function, so every existing `oha …` line in
# the scripts picks it up unchanged.
oha() {
  if [ -n "${CLIENT_CPUS:-}" ]; then taskset -c "$CLIENT_CPUS" command oha "$@"
  else command oha "$@"; fi
}

# Prefix for a stack's launch command: `on_server_cpus cmd args…`.
on_server_cpus() {
  if [ -n "${SERVER_CPUS:-}" ]; then taskset -c "$SERVER_CPUS" "$@"
  else "$@"; fi
}

# Re-pin processes a container runtime started outside our process tree.
# Rootless Podman launches containers from a systemd scope, so the affinity of
# the `podman compose up` that asked for them is not inherited; the processes
# belong to a subordinate uid, hence sudo. No-op when SERVER_CPUS is unset.
pin_pattern() {  # $1 = pgrep -f pattern
  [ -n "${SERVER_CPUS:-}" ] || return 0
  local p
  for p in $(pgrep -f "$1" 2>/dev/null); do
    taskset -a -p -c "$SERVER_CPUS" "$p" >/dev/null 2>&1 \
      || sudo -n taskset -a -p -c "$SERVER_CPUS" "$p" >/dev/null 2>&1
  done
}

# The container runtime: Docker where its daemon answers, Podman otherwise.
have_docker() { command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; }
ctr() { if have_docker; then docker "$@"; else podman "$@"; fi; }

# `compose <file> <args…>`: `docker compose`, or podman-compose with the
# rootless overlay beside the file (`<file>.podman.yml` without the first
# `.yml`), which maps the container user onto ours (`keep-id`) so php-fpm can
# write the bind-mounted storage/ — the job Docker's numeric `user:` did on the
# original box.
compose() {
  local file=$1; shift
  if have_docker; then
    docker compose -f "$file" "$@"
  else
    podman-compose -f "$file" -f "${file%.yml}.podman.yml" "$@"
  fi
}
