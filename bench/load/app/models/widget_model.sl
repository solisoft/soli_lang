# Widget model — the data row used by the db / crud benchmark workloads.
# Collection: widgets (derived from the class name).

class Widget < Model
    # Fields: name (string), category (string), price (int), active (bool)
    validates("name", { "presence": true })

    # Fetch the first `count` rows with a DB-side LIMIT (O(count), not O(table)).
    scope("limited", fn(count) { this.limit(count) })
end
