# Database seeds — run with `soli db:seed`.
#
# This file runs first, then every file in `db/seeds/` (sorted by name).
# Generate ordered seed files with: soli db:seed generate <name>
#
# Seeds are NOT tracked and re-run every time, so make them idempotent.
# Guard with first_by / find_by instead of a blind create():
#
#   3.times do |i|
#     let email = "user\(i)@example.com"
#     User.create({ "name": "User \(i)", "email": email }) if User.first_by("email", email).nil?
#   end

print("Seeded database")
