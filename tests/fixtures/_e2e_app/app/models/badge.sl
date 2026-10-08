# Not persisted. `as_json` leaves `token` out: `render_json(badge)` must send
# only what it returns, on the VM as on the tree-walker (SEC-013b).
class Badge
  label: String
  token: String

  new(label: String, token: String)
    @label = label
    @token = token
  end

  def as_json
    {"label": @label}
  end
end
