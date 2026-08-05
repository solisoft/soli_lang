defmodule BenchWeb.Post do
  @moduledoc """
  The 50-row read dataset.

  The table is created and seeded by the shared harness, so there is no
  migration here — this schema declares a mapping over a table that already
  exists, the analogue of Django's `managed = False`.
  """
  use Ecto.Schema

  schema "posts" do
    field :title, :string
    field :views, :integer
  end
end
