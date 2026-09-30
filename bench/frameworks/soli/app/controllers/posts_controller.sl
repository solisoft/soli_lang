# Three matched workloads over the same 50 records.
#   /json     -> 50 in-memory objects, serialised to JSON
#   /template -> the same 50 objects, through the ERB engine + layout
#   /db       -> the same 50 objects, read from SoliDB, serialised to JSON
# The differences between rows are therefore the template engine and the database.
# WPOOL (default 800,000) is the size of the write table, read once at boot — the
# delete row needs a pool no stack can exhaust in a 30s cell.
const WPOOL = int(getenv("WPOOL") || "800000")

class PostsController < Controller
  # GET /json — the 50 rows, serialised to JSON
  def json_only
    render_json(@_rows)
  end

  # GET /template — the same rows through the view and its layout
  def template_only
    render("posts/list", { "title": "Posts", "items": @_rows })
  end

  # GET /db — the 50 rows read from SoliDB (projected on the database side)
  def db_json
    render_json(Post.pluck(:id, :title, :views).all)
  end

  # GET /db-template — a database read, then an HTML render: /db and /template
  # in one request, the row closest to a real server-rendered page.
  def db_template
    render("posts/list", { "title": "Posts", "items": Post.pluck(:id, :title, :views).all })
  end

  # Not an action: the `_` keeps it out of the router.
  def _rows()
    (1..51).map { |i| { "id": i, "title": "Post title #{i}", "views": i * 7 } }
  end

  # ---- Ecritures : une operation par requete, sur `wposts` (800 000 docs) ----
  # Cle tiree au hasard dans le meme intervalle 1..800000 pour les trois stacks,
  # donc chaque requete adresse une ligne par sa cle primaire.
  def w_key
    return str(int(Math.random() * WPOOL) + 1)
  end

  def w_create(req: Any) -> Any {
    Wpost.create({ "title": "Post title 0", "views": 7 })
    return { "status": 201, "body": "ok" }
  }

  def w_update(req: Any) -> Any {
    Wpost.update(this.w_key(), { "views": 42 })
    return { "status": 200, "body": "ok" }
  }

  # Une cle deja supprimee est un miss : le taux de reussite est mesure en
  # comptant les documents avant/apres la cellule, pas devine.
  def w_delete(req: Any) -> Any {
    Wpost.delete(this.w_key()) rescue null
    return { "status": 200, "body": "ok" }
  }

  # Un seul document par cle — la charge mesuree separement a ~40k req/s,
  # a comparer au scan de 50 lignes de db_json.
  def find_one
    return render_json(Post.find("019fa2fb-c37a-7ce1-a7f6-f23b25b18fdc"))
  end
end
