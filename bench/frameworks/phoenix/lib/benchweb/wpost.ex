defmodule BenchWeb.Wpost do
  @moduledoc "Isolated 800,000-row table for the write workloads."
  use Ecto.Schema

  schema "wposts" do
    field :title, :string
    field :views, :integer
  end
end
