class DemoUser < Model
  validates("name", { "presence": true })
  validates("email", { "presence": true, "uniqueness": true })

  static def search(q)
    return DemoUser.order("name", "asc").all if q.nil? || q == ""

    let needle = q.downcase()
    DemoUser.where(
      "CONTAINS(LOWER(doc.name), @needle) || CONTAINS(LOWER(doc.email), @needle)",
      { "needle": needle }
    ).order("name", "asc").all
  end
end
