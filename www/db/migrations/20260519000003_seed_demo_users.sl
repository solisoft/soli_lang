def up(db: Any) -> Any {
  let first_names = ["Alice","Bob","Charlie","Diana","Edward","Fiona","George","Hannah","Ivan","Julia","Kevin","Laura","Michael","Nina","Oscar","Penny","Quinn","Rachel","Sam","Tina","Uma","Victor","Wendy","Xander","Yara","Zack"]
  let last_names = ["Smith","Johnson","Williams","Brown","Jones","Garcia","Miller","Davis","Rodriguez","Martinez","Anderson","Taylor","Thomas","Jackson","White","Harris","Clark","Lewis","Walker","Hall"]
  let roles = ["Admin","Editor","Viewer","Contributor"]
  let statuses = ["Active","Inactive"]
  let fn_len = first_names.length()
  let ln_len = last_names.length()
  let r_len = roles.length()
  let s_len = statuses.length()
  let batch = []
  let total = 3000
  for i in 0..total
    let fi = i % fn_len
    let li = (i + i / fn_len) % ln_len
    let ri = i % r_len
    let si = (i + 7) % s_len
    let name = "#{first_names[fi]} #{last_names[li]}"
    let h = 8 + (i % 12)
    let m = (i * 7) % 60
    batch.push({
      "name": name,
      "email": "#{first_names[fi].downcase()}.#{last_names[li].downcase()}.#{i + 1}@demo.soli",
      "role": roles[ri],
      "status": statuses[si],
      "last_login": "2026-05-19 #{h < 10 ? '0' : ''}#{h}:#{m < 10 ? '0' : ''}#{m}"
    })
    if batch.length() == 500 || i == total - 1
      let json = json_stringify(batch)
      let q = "FOR doc IN " + json + " INSERT doc INTO demo_users"
      solidb_query(_db, q)
      batch = []
    end
  end
}

def down(db: Any) -> Any {
  solidb_query(_db, "FOR doc IN demo_users FILTER CONTAINS(doc.email, '@demo.soli') REMOVE doc IN demo_users")
}
