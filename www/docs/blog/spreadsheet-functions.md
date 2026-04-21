# Importing CSV & Excel Files in Soli

Soli makes working with spreadsheet data straightforward. Whether you're importing user data from a CSV export, processing Excel reports, or building a data pipeline, the built-in spreadsheet functions have you covered.

## Parsing CSV Data

CSV (Comma-Separated Values) is the most common format for data exchange. Soli provides two functions: one for parsing CSV strings and another for reading files directly.

### Parsing a CSV String

When your CSV data comes from an API response or a text field:

```soli
csv_data = "name,email,role\nAlice,alice@example.com,admin\nBob,bob@example.com,user"
users = csv_parse(csv_data)

print(users[0]["name"])  # "Alice"
print(users[1]["role"])  # "user"
```

The `csv_parse()` function returns an array of hashes, using the first row as keys.

### Reading CSV Files

When your data is in a file, use `csv_parse_file()`:

```soli
# Import users from a CSV export
users = csv_parse_file("exports/users.csv")

for user in users
    u = User.create({
        "name": user["name"],
        "email": user["email"],
        "role": user["role"]
    })
    print("Created: " + u.email)
end
```

## Processing Excel Files

The `excel_parse()` function reads `.xlsx` files directly, making it easy to process Excel reports:

```soli
# Read a sales report
sales = excel_parse("reports/monthly_sales.xlsx")

for row in sales
    print(row["Product"] + ": $" + str(row["Revenue"]))
end
```

### Real-World Example: User Import with Validation

Here's a complete example that imports users from a CSV file, validates the data, and creates user records:

```soli
def import_users_from_csv(filepath)
    users = csv_parse_file(filepath)
    imported = 0
    errors = []

    for row in users
        # Validate required fields
        if !row["email"] || !row["name"]
            errors.push("Row missing required field: " + str(row))
            next
        end

        # Check for existing user
        if User.find_by("email", row["email"])
            print("Skipping existing user: " + row["email"])
            next
        end

        # Create the user
        user = User.create({
            "name": row["name"],
            "email": row["email"],
            "role": row["role"] || "user"
        })

        imported = imported + 1
    end

    return {
        "imported": imported,
        "errors": errors
    }
end

# Usage
result = import_users_from_csv("data/new_users.csv")
print("Imported " + str(result["imported"]) + " users")
```

### Real-World Example: Sales Report Processing

Process monthly Excel reports to generate summaries:

```soli
def process_sales_report(filepath)
    data = excel_parse(filepath)
    total_revenue = 0
    by_category = {}

    for row in data
        revenue = float(row["Revenue"])
        category = row["Category"]

        total_revenue = total_revenue + revenue

        if !by_category.has_key(category)
            by_category[category] = 0
        end
        by_category[category] = by_category[category] + revenue
    end

    print("Total Revenue: $" + str(total_revenue))
    print("\nBy Category:")
    for category, total in by_category
        print("  " + category + ": $" + str(total))
    end
end

process_sales_report("reports/q4_sales.xlsx")
```

### Real-World Example: Data Migration

Migrate data from a legacy system exported as CSV to your Soli app:

```soli
def migrate_products_from_legacy(csv_path)
    products = csv_parse_file(csv_path)
    migrated = 0

    for row in products
        # Skip if product already exists by SKU
        if Product.find_by("sku", row["sku"])
            next
        end

        product = Product.create({
            "sku": row["sku"],
            "name": row["product_name"],
            "price": float(row["price"]),
            "stock": int(row["quantity"]),
            "category": row["category"]
        })

        migrated = migrated + 1
    end

    print("Migrated " + str(migrated) + " products")
end
```

## Summary

| Function | Description |
|----------|-------------|
| `csv_parse(str)` | Parse a CSV string into an array of hashes |
| `csv_parse_file(path)` | Read and parse a CSV file |
| `excel_parse(path)` | Read and parse an Excel (.xlsx) file |

These functions make it trivial to integrate spreadsheet data into your Soli applications. Whether you're building import tools, processing reports, or migrating data, the syntax stays simple and the code stays readable.