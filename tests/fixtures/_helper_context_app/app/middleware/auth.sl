# order: 10
def load_user(req)
  req["current_user"] = req["headers"]["x-user"]
  req
end
