# SoliDB round-trip benchmark — how long a hundred of each verb takes.
#
# Needs a SoliDB listening on the host below; it exits with a message rather
# than a stack trace when there is none, so it is safe to run in a checkout
# that has no server. Point it elsewhere with SOLIDB_HOST / SOLIDB_DATABASE.
#
# This file used to be written against `new Solidb(...)`, `println(...)`,
# `end` as a variable name and `Duration.total_millis()` — a constructor form,
# a builtin, a keyword and a method, none of which Soli has. It had presumably
# never run, because nothing executed `examples/` until `check-examples.sh`
# started parsing them.
#
#   soli examples/solidb_bench.sl

const ROUNDS = 100

let host     = getenv("SOLIDB_HOST") ?? "http://localhost:6745"
let database = getenv("SOLIDB_DATABASE") ?? "solidb"

let db = Solidb(host, database)

# `ping` raises rather than answering false when the server refuses — an
# unreachable host and a 401 both arrive that way — so the check rescues.
let reachable = db.ping() rescue false

# Each measurement is the same shape: run the verb ROUNDS times, and report the
# span and what one call cost. `Duration.between` is what measures it, and
# `DateTime.now()` carries a subsecond, so a hundred fast calls do not all
# round to zero.
def measure(label, work) {
    let started = DateTime.now()
    for i in 1..ROUNDS {
        work()
    }
    let elapsed = Duration.between(started, DateTime.now()).total_seconds()
    let each_ms = (elapsed / ROUNDS) * 1000.0
    print("#{label.ljust(24)} #{elapsed}s total, #{each_ms}ms each")
}

if reachable
    print("SoliDB round-trip benchmark — #{ROUNDS} calls per verb, against #{host}")
    print("----------------------------------------------------------------")
    measure("ping", fn() db.ping())
    measure("query (LIMIT 1)", fn() db.query("FOR u IN users LIMIT 1 RETURN u", {}))
    measure("get by key", fn() db.get("users", "user1") rescue null)
    print("Done.")
else
    print("No SoliDB answering at #{host}, or it refused us — nothing to measure.")
    print("Start one, or set SOLIDB_HOST / SOLIDB_DATABASE, and run this again.")
end
