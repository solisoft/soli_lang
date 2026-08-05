defmodule BenchWebWeb.BenchHTML do
  @moduledoc "Templates for the matched HTML workloads."
  use BenchWebWeb, :html

  embed_templates "bench_html/*"
end
