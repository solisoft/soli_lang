# Metrics — engine-embedded Soli stdlib (see builtins::metrics::register_metrics_class).
#
# App metrics, shared by every worker, exposed at /_metrics (Prometheus),
# sent to a Datadog agent (DogStatsD) and/or exported over OTLP — whichever
# is configured. With none configured a sample is validated and dropped
# (except under `soli test`, where it is kept so a spec can read it back).
#
#   Metrics.increment("orders.created", tags: {"plan": "pro"})
#   Metrics.gauge("queue.depth", jobs.length)
#   Metrics.histogram("cart.items", cart.items.length)
#   Metrics.time("checkout.duration", {"step": "pay"}) do
#     charge(order)
#   end
#
# Plain natives cannot run a block, so the class lives here in Soli.

class Metrics
  static def increment(name, value = 1, tags = {})
    __metrics_record("counter", "c", name, value, tags)
  end

  static def gauge(name, value, tags = {})
    __metrics_record("gauge", "g", name, value, tags)
  end

  static def histogram(name, value, tags = {})
    __metrics_record("histogram", "h", name, value, tags)
  end

  # A histogram sent to DogStatsD as a distribution (`|d`).
  static def distribution(name, value, tags = {})
    __metrics_record("histogram", "d", name, value, tags)
  end

  # Run the block, record how long it took in seconds (a distribution, also
  # when it raises), and answer its value: `Metrics.time("x") do … end`,
  # `Metrics.time("x", {"step": "pay"}) do … end`. Tags go positionally
  # here: the VM refuses a labelled call that also carries a block.
  #
  # `block` is only ever called, never read bare: on the tree-walker a bare
  # read of a zero-parameter function runs it. The VM binds an unlabelled
  # `do` block to `tags` instead of `&block` (and never runs a bare read), so
  # a function found there is the block.
  static def time(name, tags = {}, &block)
    return Metrics._time_block(name, tags) if __metrics_callable(tags)

    started = __metrics_clock()
    result = nil
    try
      result = block()
    catch error
      __metrics_record("histogram", "d", name, __metrics_clock() - started, tags)
      throw error
    end
    __metrics_record("histogram", "d", name, __metrics_clock() - started, tags)
    result
  end

  # `time` for a block the VM bound to `tags` (reached on the VM only).
  static def _time_block(name, block)
    started = __metrics_clock()
    result = nil
    try
      result = block()
    catch error
      __metrics_record("histogram", "d", name, __metrics_clock() - started, {})
      throw error
    end
    __metrics_record("histogram", "d", name, __metrics_clock() - started, {})
    result
  end

  # The current value of one series: a number for a counter or a gauge,
  # {"count", "sum", "buckets"} for a histogram, nil when never recorded.
  static def value(name, tags = {})
    __metrics_value(name, tags)
  end

  static def enabled?
    __metrics_enabled()
  end

  # Forget every app metric. Test runs only (`APP_ENV=test`).
  static def reset
    __metrics_reset()
  end
end
