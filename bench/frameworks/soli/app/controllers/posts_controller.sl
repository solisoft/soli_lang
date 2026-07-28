# Three matched workloads over the same 50 records.
#   /json     -> 50 in-memory objects, serialised to JSON
#   /template -> the same 50 objects, through the ERB engine + layout
#   /db       -> the same 50 objects, read from SoliDB, serialised to JSON
# The differences between rows are therefore the template engine and the database.
class PostsController < Controller
  def rows
    return (0..50).map(fn(i) { return { "id": i + 1, "title": "Post title #{i + 1}", "views": (i + 1) * 7 } })
  end

  def json_only
    render_json(this.rows())
  end

  def template_only
    render("posts/list", { "title": "Posts", "items": this.rows() })
  end

  def db_json
    # Projection cote base : seuls les champs demandes traversent le reseau.
    # La forme en ligne est correcte depuis le correctif de render_json (le
    # builder n'est plus evalue deux fois) ; la locale reste pour que les
    # chiffres publies correspondent exactement au code mesure.
    let rows = Post.pluck(:id, :title, :views).all
    return render_json(rows)
  end

  # La page rendue serveur telle qu'on l'ecrit vraiment : lecture DB puis rendu
  # HTML. C'est /db et /template dans une seule requete, donc le cout des deux
  # plus rien d'autre — la ligne la plus representative de la page reelle.
  def db_template
    render("posts/list", { "title": "Posts", "items": Post.pluck(:id, :title, :views).all })
  end

  # ---- Ecritures : une operation par requete, sur `wposts` (800 000 docs) ----
  # Cle tiree au hasard dans le meme intervalle 1..800000 pour les trois stacks,
  # donc chaque requete adresse une ligne par sa cle primaire.
  def w_key
    return str(int(Math.random() * 800000) + 1)
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
