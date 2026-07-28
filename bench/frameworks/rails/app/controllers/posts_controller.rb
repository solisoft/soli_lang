# Three matched workloads over the same 50 records — mirrors the Soli app exactly.
class PostsController < ApplicationController
  layout "application"
  # The load generator carries no session or CSRF token; the write routes are a
  # benchmark fixture, not a form endpoint.
  skip_forgery_protection

  def rows
    (1..50).map { |i| { id: i, title: "Post title #{i}", views: i * 7 } }
  end

  def json_only
    render json: rows
  end

  def template_only
    @title = "Posts"
    render "posts/list", locals: { items: rows }
  end

  def db_json
    render json: Post.pluck(:id, :title, :views).map { |id, title, views| { id: id, title: title, views: views } }
  end

  # Fairness probe: pluck returns arrays, so the payload shape costs Rails a
  # Ruby-side map. select_all is the true analogue of node-postgres' pool.query
  # — the adapter builds the hashes, as the C driver does for Express.
  def db_json_select_all
    render json: Post.connection.select_all("SELECT id, title, views FROM posts").to_a
  end

  # ---- Writes: one operation per request, on `wposts` (800,000 rows) ----
  # Random key over the same 1..800000 range as the other two stacks, so every
  # request addresses one row by primary key. update_all/delete_all are the
  # ORM's single-statement forms — the analogue of Soli's Model.update/delete.
  WPOOL = 800_000

  def w_create
    Wpost.create!(title: "Post title 0", views: 7)
    head :created
  end

  def w_update
    Wpost.where(id: rand(1..WPOOL)).update_all(views: 42)
    head :ok
  end

  def w_delete
    Wpost.where(id: rand(1..WPOOL)).delete_all
    head :ok
  end

  # Read from the database, then render HTML — /db and /template in one request.
  def db_template
    @title = "Posts"
    render "posts/list", locals: {
      items: Post.pluck(:id, :title, :views).map { |id, title, views| { id: id, title: title, views: views } }
    }
  end
end
