# One press of the button on /d1: the datacenter that answered and the
# visitor's country, as Cloudflare reported them. Nothing that identifies the
# visitor is stored. On the Worker it lives in D1; locally, in SQLite.
class CheckIn < Model
  validates("colo", {"presence": true, "max_length": 8})
  validates("country", {"presence": true, "max_length": 8})
end
