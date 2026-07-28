# Regression probe for argument double-evaluation.
#
# `render_json(expr)` used to evaluate `expr` twice: the interceptor
# implementing the `as_json` override evaluated the first argument to test
# whether it was an instance, then discarded the value and returned None for
# anything else, so normal dispatch evaluated it again. For a query builder
# (`render_json(Post.pluck(...).all)`) that issued the database query twice on
# every request.
class EvalsController < Controller
  def counted
    this.n = (this.n ?? 0) + 1
    return { "n": this.n }
  end

  # The body reports how many times the argument was evaluated:
  # {"n":1} once, {"n":2} twice.
  def render_json_arg_evals(req: Any) -> Any {
    this.n = 0
    return render_json(this.counted())
  }
end
