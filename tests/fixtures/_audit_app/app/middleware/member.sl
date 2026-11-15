# order: 5

def load_member(req)
  email = req["headers"]["x-member"] rescue nil
  req["current_user"] = Member.find_by("email", email) unless email.blank?
  req
end
