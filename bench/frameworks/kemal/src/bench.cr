# Kemal + Jennifer + ECR + PostgreSQL — the seven matched workloads.
#
# Kemal has neither an ORM nor a view layer beyond ECR: Jennifer is added, the
# way Sequelize is added to Express, and the column is named after both. One
# process per core: `start-bench.sh` starts 16 of these on the same port with
# SO_REUSEPORT, each single-threaded — Crystal's multi-threading is still
# behind `-Dpreview_mt`, and a fleet of processes is how Kemal is deployed.
# A pool of 5 per process makes 80 connections, the budget every stack gets.
require "kemal"
require "jennifer"
require "jennifer/adapter/postgres"

PORT  = (ENV["PORT"]? || "5105").to_i
WPOOL = (ENV["WPOOL"]? || "800000").to_i

Jennifer::Config.configure do |conf|
  conf.adapter = "postgres"
  conf.host = "127.0.0.1"
  conf.port = 5433
  conf.user = "bench"
  conf.password = "bench"
  conf.db = "bench"
  conf.pool_size = 5
  conf.max_pool_size = 5
  conf.initial_pool_size = 5
  conf.max_idle_pool_size = 5
  # Jennifer's own logger defaults to DEBUG and writes every query; the other
  # stacks log no queries in production.
  conf.logger = Log.for("db", :error)
end
Log.setup(:error)

class Post < Jennifer::Model::Base
  mapping(id: Primary32, title: String?, views: Int32?)
end

class Wpost < Jennifer::Model::Base
  table_name "wposts"
  mapping(id: Primary32, title: String?, views: Int32?)
end

alias Row = NamedTuple(id: Int32, title: String, views: Int32)

# 50 in-memory rows, identical to the other stacks.
def rows : Array(Row)
  Array(Row).new(50) { |i| {id: i + 1, title: "Post title #{i + 1}", views: (i + 1) * 7} }
end

# Projection without instantiating models — Jennifer's `pluck`, the analogue
# of Rails' and Soli's pluck, Sequelize's raw:true and Django's .values().
def db_rows : Array(Row)
  Post.all.pluck(:id, :title, :views).map do |r|
    {id: r[0].as(Int32), title: r[1].as(String), views: r[2].as(Int32)}
  end
end

def page(items : Array(Row)) : String
  title = "Posts"
  ECR.render "src/views/list.ecr"
end

def json(env, body : String)
  env.response.content_type = "application/json"
  body
end

get "/json" { |env| json(env, rows.to_json) }
get "/template" { |env| env.response.content_type = "text/html"; page(rows) }
get "/db" { |env| json(env, db_rows.to_json) }
get "/db-template" { |env| env.response.content_type = "text/html"; page(db_rows) }

# Writes, one operation per request on `wposts`, keys drawn from 1..WPOOL like
# the other stacks.
post "/w" do |env|
  Wpost.create(title: "Post title 0", views: 7)
  env.response.status_code = 201
  ""
end
patch "/w" do
  Wpost.where { _id == Random.rand(1..WPOOL) }.update({:views => 42})
  ""
end
delete "/w" do
  Wpost.where { _id == Random.rand(1..WPOOL) }.delete
  ""
end

Kemal.config.env = "production"
Kemal.config.logging = false
Kemal.run do |config|
  config.server.not_nil!.bind_tcp("127.0.0.1", PORT, reuse_port: true)
end
