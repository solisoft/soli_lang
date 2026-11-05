# order: 20
def gate(req)
  return render("home/blocked") if req["path"] == "/blocked"

  req
end
