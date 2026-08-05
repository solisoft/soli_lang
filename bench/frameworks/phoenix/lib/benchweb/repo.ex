defmodule BenchWeb.Repo do
  use Ecto.Repo,
    otp_app: :benchweb,
    adapter: Ecto.Adapters.Postgres
end
