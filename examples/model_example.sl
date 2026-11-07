# Model/ORM Example
# How a Soli model reads and writes documents. Run it inside an application
# (`soli serve` or `soli test`), where the database connection is configured;
# a model class is all it takes — `create`, `find`, `where` and the rest are
# inherited from Model.

class User < Model
  validates("name", { "presence": true })
  validates("email", { "presence": true, "uniqueness": true })
end

# Create a user
user = User.create({ "name": "Alice", "email": "alice@example.com", "age": 30 })

# Query users with a filter, ordered and limited
adults = User.where("doc.age >= @age", { "age": 18 }).order("name", "asc").limit(10).all

# Get a specific user by key (raises RecordNotFound on a miss; find_by returns nil)
same_user = User.find(user._key)
by_email = User.find_by("email", "alice@example.com")

# Update a user
same_user.update({ "age": 31 })

# Count users
count = User.count

# Delete a user
User.delete(user._key)

# Migrations live in db/migrations/ and run with `soli db:migrate`.
print("Model example completed! #{adults.length} adult(s), #{count} user(s)")
