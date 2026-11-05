def current_user_name()
  return "guest" if req.nil?

  user = req["current_user"]
  user.nil? ? "guest" : user
end
