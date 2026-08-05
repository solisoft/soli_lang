defmodule BenchWebWeb.BenchController do
  @moduledoc """
  The same seven matched workloads as the Soli, Rails, Express, AdonisJS,
  Laravel, Django and FastAPI apps.

  The DB rows project **in the database** without loading schema structs —
  `select: %{id: ..., title: ..., views: ...}` is Ecto's analogue of Rails'
  `pluck`, Soli's `pluck`, Sequelize's `raw: true`, Eloquent's `toBase()`,
  Django's `.values()` and SQLAlchemy's `select(Post.id, ...)`. `/db-hydrated`
  is the reference form that does build 50 structs.
  """
  use BenchWebWeb, :controller

  import Ecto.Query

  alias BenchWeb.{Post, Repo, Wpost}

  @wpool 800_000

  # Projection without loading structs. Built once at compile time.
  @posts from(p in Post, select: %{id: p.id, title: p.title, views: p.views})

  defp rows do
    Enum.map(1..50, fn i -> %{id: i, title: "Post title #{i}", views: i * 7} end)
  end

  defp db_rows, do: Repo.all(@posts)

  def json_only(conn, _params), do: json(conn, rows())

  def db_json(conn, _params), do: json(conn, db_rows())

  def db_hydrated(conn, _params) do
    # Reference: the form that does materialise 50 schema structs.
    json(conn, Enum.map(Repo.all(Post), &%{id: &1.id, title: &1.title, views: &1.views}))
  end

  # The template is the whole document, matching the Django, Laravel and FastAPI
  # apps, so the byte counts stay comparable.
  #
  # BOTH layouts have to be turned off. `layout: false` only disables the inner
  # app layout; the router's `put_root_layout` is separate, and leaving it on
  # wrapped this document inside Phoenix's root layout — a second <!DOCTYPE html>
  # nested in <body>, and 3,492 bytes of it. That renders in a browser without
  # complaint, which is exactly why it has to be checked rather than assumed.
  defp bare(conn), do: conn |> put_root_layout(html: false) |> put_layout(html: false)

  def template_only(conn, _params) do
    conn |> bare() |> render(:list, title: "Posts", items: rows())
  end

  def db_template(conn, _params) do
    conn |> bare() |> render(:list, title: "Posts", items: db_rows())
  end

  # ---- Writes: one operation per request, against `wposts` (800,000 rows) ----
  # The key is drawn from the same 1..800000 range as every other stack, so each
  # request addresses one row by primary key.

  defp wkey, do: :rand.uniform(@wpool)

  def w_create(conn, _params) do
    Repo.insert!(%Wpost{title: "Post title 0", views: 7})
    send_resp(conn, 201, "")
  end

  def w_update(conn, _params) do
    from(w in Wpost, where: w.id == ^wkey()) |> Repo.update_all(set: [views: 42])
    send_resp(conn, 200, "")
  end

  def w_delete(conn, _params) do
    from(w in Wpost, where: w.id == ^wkey()) |> Repo.delete_all()
    send_resp(conn, 200, "")
  end
end
