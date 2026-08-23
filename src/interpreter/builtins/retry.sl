# Retry — engine-embedded Soli stdlib (see builtins::retry::register_retry_class).
#
# Plain natives cannot invoke Soli functions, so retry logic lives here in
# Soli itself: `Retry.with_backoff(block, opts)` runs `block()` up to
# `attempts` times, sleeping with exponential backoff between attempts.
#
#   let result = Retry.with_backoff(fn() {
#       HTTP.post_json(url, body)
#   }, { "attempts": 3, "base_delay": 0.5 });
#
# Options:
#   attempts    total tries including the first   (default 3)
#   base_delay  seconds before the first retry     (default 0.5)
#   max_delay   cap for the exponential backoff    (default 8)
#   factor      multiplier per attempt             (default 2)
#
# The last error is re-raised when every attempt fails.

class Retry
    static def with_backoff(block, opts = {})
        # `??` (nil-coalescing), not `||`: with `||` a supplied 0 is falsey and
        # silently became the default, so `{"attempts": 0}` ran 3 times and
        # `{"base_delay": 0}` slept 0.5s. Config-driven zeros are ordinary.
        let attempts = opts["attempts"] ?? 3;
        let base_delay = opts["base_delay"] ?? 0.5;
        let max_delay = opts["max_delay"] ?? 8;
        let factor = opts["factor"] ?? 2;

        # Now that 0 survives, say what is out of range rather than throwing a
        # null `last_error` from a loop that never ran.
        if attempts < 1
            throw "Retry.with_backoff(): \"attempts\" must be >= 1, got " + str(attempts);
        end

        let delay = base_delay;
        let attempt = 0;
        let last_error = null;

        while attempt < attempts {
            try {
                return block();
            } catch e {
                last_error = e;
                attempt = attempt + 1;
                if attempt < attempts {
                    sleep(delay);
                    let next = delay * factor;
                    if next > max_delay {
                        next = max_delay;
                    }
                    delay = next;
                }
            }
        }

        throw last_error;
    end

    # Run once; on failure keep retrying until the deadline passes rather
    # than a fixed count. Useful for "be ready within N seconds" boot work:
    #
    #   Retry.within(fn() { Solidb.ping() }, { "deadline": 10 });
    #
    # Options:
    #   deadline    seconds to keep trying             (default 10)
    #   base_delay  seconds before the first retry     (default 0.5)
    #   max_delay   cap for the exponential backoff    (default 8)
    #   factor      multiplier per attempt             (default 2)
    #
    # Backs off the same way `with_backoff` does; it just bounds the work by
    # elapsed time instead of a try count. It previously ignored factor and
    # max_delay entirely and retried at a constant 0.25s, so a long deadline
    # meant hundreds of tight retries against a service that was already down.
    static def within(block, opts = {})
        # See `with_backoff`: `??` so a supplied 0 means 0. `{"deadline": 0}`
        # used to wait the full default 10 seconds.
        let deadline_secs = opts["deadline"] ?? 10;
        let base_delay = opts["base_delay"] ?? 0.5;
        let max_delay = opts["max_delay"] ?? 8;
        let factor = opts["factor"] ?? 2;

        let delay = base_delay;
        let started = DateTime.microtime();
        let last_error = null;

        while true {
            try {
                return block();
            } catch e {
                last_error = e;
                # Sleep only as long as the deadline still allows. Checking the
                # deadline and then sleeping a full `delay` overshot it by up to
                # one delay: `{"deadline": 2, "base_delay": 1.5}` took 4.5s, and
                # with the default max_delay of 8 a failing check could block
                # ~18s for a 10s deadline — the opposite of "ready within N".
                let remaining = deadline_secs * 1000000.0 - (DateTime.microtime() - started)
                if remaining <= 0 {
                    throw last_error;
                }
                let nap = delay
                if nap * 1000000.0 > remaining {
                    nap = remaining / 1000000.0
                }
                sleep(nap);
                let next = delay * factor;
                if next > max_delay {
                    next = max_delay;
                }
                delay = next;
            }
        }
    end
end
