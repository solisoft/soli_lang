// ============================================================================
// Destructuring Examples for AI/LLM Code Generation
// ============================================================================
//
// AI AGENT GUIDE:
// ---------------
// This file demonstrates destructuring syntax in Soli.
// Use destructuring to extract values from hashes and arrays.
//
// HASH DESTRUCTURING:
// let { key1, key2 } = hash_object;
// let { key1, key2 = "default" } = hash_object;  // With default value
// let { nested: { inner } } = hash_object;        // Nested destructuring
//
// ARRAY DESTRUCTURING:
// let [first, second] = array;
// let [first, ...rest] = array;                   // Rest operator
// let [first, , third] = array;                   // Skip elements
//
// COMBINED:
// let { user: { name, email }, posts } = data;
//
// ============================================================================

// ============================================================================
// EXAMPLE 1: Basic Hash Destructuring
// ============================================================================

class HashDestructuringController extends Controller {
    fn basic_example(req: Any) -> Any {
        let user = {
            "id": 1,
            "name": "John Doe",
            "email": "john@example.com",
            "age": 30
        };

        let { id, name, email } = user;

        return {
            "status": 200,
            "body": json_stringify({
                "id": id,
                "name": name,
                "email": email
            })
        };
    }

    fn with_defaults(req: Any) -> Any {
        let user = {
            "id": 1,
            "name": "John"
        };

        let { id, name, email = "unknown@example.com", age = 0 } = user;

        return {
            "status": 200,
            "body": json_stringify({
                "id": id,
                "name": name,
                "email": email,
                "age": age
            })
        };
    }

    fn partial_destructuring(req: Any) -> Any {
        let post = {
            "id": 1,
            "title": "Hello World",
            "content": "This is a post",
            "author": "John",
            "created_at": "2024-01-15",
            "tags": ["tech", "programming"]
        };

        let { title, author } = post;

        return {
            "status": 200,
            "body": json_stringify({
                "title": title,
                "author": author
            })
        };
    }
}

// ============================================================================
// EXAMPLE 2: Array Destructuring
// ============================================================================

class ArrayDestructuringController extends Controller {
    fn basic_example(req: Any) -> Any {
        let coordinates = [10, 20, 30];

        let [x, y, z] = coordinates;

        return {
            "status": 200,
            "body": json_stringify({
                "x": x,
                "y": y,
                "z": z
            })
        };
    }

    fn with_rest(req: Any) -> Any {
        let numbers = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

        let [first, second, ...rest] = numbers;

        return {
            "status": 200,
            "body": json_stringify({
                "first": first,
                "second": second,
                "rest": rest,
                "rest_count": len(rest)
            })
        };
    }

    fn skip_elements(req: Any) -> Any {
        let date = [2024, 1, 15, 10, 30, 45];

        let [year, month, day] = date;

        return {
            "status": 200,
            "body": json_stringify({
                "year": year,
                "month": month,
                "day": day
            })
        };
    }

    fn head_tail(req: Any) -> Any {
        let items = ["apple", "banana", "cherry", "date", "elderberry"];

        let [head, ...tail] = items;

        return {
            "status": 200,
            "body": json_stringify({
                "head": head,
                "tail": tail,
                "tail_length": len(tail)
            })
        };
    }
}

// ============================================================================
// EXAMPLE 3: Nested Destructuring
// ============================================================================

class NestedDestructuringController extends Controller {
    fn nested_hash(req: Any) -> Any {
        let user = {
            "id": 1,
            "name": "John Doe",
            "profile": {
                "bio": "Software developer",
                "location": "New York",
                "joined": "2020-01-15"
            },
            "settings": {
                "notifications": true,
                "theme": "dark"
            }
        };

        let { name, profile: { bio, location }, settings: { theme } } = user;

        return {
            "status": 200,
            "body": json_stringify({
                "name": name,
                "bio": bio,
                "location": location,
                "theme": theme
            })
        };
    }

    fn mixed_nested(req: Any) -> Any {
        let data = {
            "user": {
                "id": 1,
                "name": "John",
                "email": "john@example.com"
            },
            "posts": [
                { "id": 1, "title": "Post 1" },
                { "id": 2, "title": "Post 2" }
            ],
            "metadata": {
                "total_posts": 2,
                "page": 1
            }
        };

        let { user: { name, email }, posts, metadata: { total_posts } } = data;

        return {
            "status": 200,
            "body": json_stringify({
                "user": { "name": name, "email": email },
                "posts_count": len(posts),
                "total_posts": total_posts
            })
        };
    }

    fn deep_nesting(req: Any) -> Any {
        let config = {
            "app": {
                "name": "MyApp",
                "version": "1.0.0",
                "database": {
                    "host": "localhost",
                    "port": 6745,
                    "name": "myapp"
                }
            }
        };

        let { app: { name, database: { host, port } } } = config;

        return {
            "status": 200,
            "body": json_stringify({
                "app": name,
                "database": { "host": host, "port": port }
            })
        };
    }
}

// ============================================================================
// EXAMPLE 4: Destructuring in Function Parameters
// ============================================================================

class FunctionParameterController extends Controller {
    fn process_user(req: Any) -> Any {
        let user = req["json"];

        let result = create_user_profile(user);

        return {
            "status": 200,
            "body": json_stringify(result)
        };
    }

    fn create_user_profile(user: Hash) -> Any {
        let { name, email, age = 18, city = "Unknown" } = user;

        return {
            "name": name,
            "email": email,
            "age": age,
            "city": city,
            "welcome": "Hello, " + name + "!"
        };
    }

    fn calculate_distance(req: Any) -> Any {
        let point = req["json"]["point"];

        let [x, y] = point;

        return {
            "status": 200,
            "body": json_stringify({
                "x": x,
                "y": y,
                "distance": sqrt(x * x + y * y)
            })
        };
    }

    fn process_order(req: Any) -> Any {
        let order = req["json"];

        let result = calculate_order_total(order);

        return {
            "status": 200,
            "body": json_stringify(result)
        };
    }

    fn calculate_order_total(order: Hash) -> Any {
        let { items, discount = 0, tax_rate = 0.1 } = order;

        let subtotal = items.reduce(0, fn(sum, item) {
            return sum + item["price"] * item["quantity"];
        });

        let discount_amount = subtotal * discount;
        let taxable = subtotal - discount_amount;
        let tax = taxable * tax_rate;
        let total = taxable + tax;

        return {
            "subtotal": subtotal,
            "discount": discount_amount,
            "tax": tax,
            "total": total
        };
    }
}

// ============================================================================
// EXAMPLE 5: Destructuring in Loops
// ============================================================================

class LoopDestructuringController extends Controller {
    fn process_users(req: Any) -> Any {
        let users = [
            { "id": 1, "name": "John", "email": "john@example.com" },
            { "id": 2, "name": "Jane", "email": "jane@example.com" },
            { "id": 3, "name": "Bob", "email": "bob@example.com" }
        ];

        let results = [];

        for { id, name, email } in users {
            results.push({
                "user_id": id,
                "display_name": name,
                "contact": email
            });
        }

        return {
            "status": 200,
            "body": json_stringify({ "users": results })
        };
    }

    fn process_coordinates(req: Any) -> Any {
        let points = [
            [0, 0],
            [10, 20],
            [30, 40],
            [50, 60]
        ];

        let sum_x = 0;
        let sum_y = 0;

        for [x, y] in points {
            sum_x = sum_x + x;
            sum_y = sum_y + y;
        }

        return {
            "status": 200,
            "body": json_stringify({
                "total_x": sum_x,
                "total_y": sum_y,
                "center": [sum_x / len(points), sum_y / len(points)]
            })
        };
    }

    fn process_nested_data(req: Any) -> Any {
        let data = [
            {
                "category": "Electronics",
                "products": [
                    { "name": "Laptop", "price": 1000 },
                    { "name": "Phone", "price": 500 }
                ]
            },
            {
                "category": "Books",
                "products": [
                    { "name": "Novel", "price": 20 },
                    { "name": "Textbook", "price": 100 }
                ]
            }
        ];

        let all_products = [];

        for { category: cat, products } in data {
            for { name, price } in products {
                all_products.push({
                    "category": cat,
                    "product": name,
                    "price": price
                });
            }
        }

        return {
            "status": 200,
            "body": json_stringify({ "products": all_products })
        };
    }
}

// ============================================================================
// EXAMPLE 6: Destructuring with Array Methods
// ============================================================================

class ArrayMethodController extends Controller {
    fn map_example(req: Any) -> Any {
        let users = [
            { "id": 1, "name": "John", "score": 85 },
            { "id": 2, "name": "Jane", "score": 92 },
            { "id": 3, "name": "Bob", "score": 78 }
        ];

        let results = users.map(fn({ id, name, score }) {
            return {
                "id": id,
                "name": name,
                "grade": score >= 90 ? "A" : score >= 80 ? "B" : "C"
            };
        });

        return {
            "status": 200,
            "body": json_stringify({ "results": results })
        };
    }

    fn filter_example(req: Any) -> Any {
        let products = [
            { "name": "Laptop", "price": 1000, "category": "Electronics" },
            { "name": "Book", "price": 20, "category": "Books" },
            { "name": "Phone", "price": 500, "category": "Electronics" },
            { "name": "Desk", "price": 200, "category": "Furniture" }
        ];

        let electronics = products.filter(fn({ category }) {
            return category == "Electronics";
        });

        return {
            "status": 200,
            "body": json_stringify({ "electronics": electronics })
        };
    }

    fn reduce_example(req: Any) -> Any {
        let orders = [
            { "id": 1, "items": [{ "price": 10 }, { "price": 20 }] },
            { "id": 2, "items": [{ "price": 15 }] },
            { "id": 3, "items": [{ "price": 25 }, { "price": 30 }] }
        ];

        let total_revenue = orders.reduce(0, fn(sum, { items }) {
            let order_total = items.reduce(0, fn(s, { price }) {
                return s + price;
            });
            return sum + order_total;
        });

        return {
            "status": 200,
            "body": json_stringify({ "total_revenue": total_revenue })
        };
    }

    fn find_example(req: Any) -> Any {
        let users = [
            { "id": 1, "name": "John", "role": "admin" },
            { "id": 2, "name": "Jane", "role": "user" },
            { "id": 3, "name": "Bob", "role": "user" }
        ];

        let admin = users.find(fn({ role }) {
            return role == "admin";
        });

        return {
            "status": 200,
            "body": json_stringify({ "admin": admin })
        };
    }
}

// ============================================================================
// EXAMPLE 7: Destructuring in Error Handling
// ============================================================================

class ErrorDestructuringController extends Controller {
    fn handle_error_response(req: Any) -> Any {
        let error_response = {
            "error": {
                "code": "VALIDATION_ERROR",
                "message": "Invalid input",
                "details": {
                    "field": "email",
                    "issue": "Invalid format"
                }
            },
            "request_id": "req_123",
            "timestamp": "2024-01-15T10:30:00Z"
        };

        let { error: { code, message, details: { field, issue } }, request_id, timestamp } = error_response;

        return {
            "status": 400,
            "body": json_stringify({
                "code": code,
                "message": message,
                "field": field,
                "issue": issue,
                "request_id": request_id
            })
        };
    }

    fn handle_api_error(req: Any) -> Any {
        let api_response = {
            "status": "error",
            "data": null,
            "error": {
                "type": "RateLimitError",
                "message": "Too many requests",
                "retry_after": 60
            }
        };

        let { status, error: { type: error_type, message, retry_after } } = api_response;

        return {
            "status": 429,
            "body": json_stringify({
                "error": error_type,
                "message": message,
                "retry_after": retry_after
            })
        };
    }
}

// ============================================================================
// EXAMPLE 8: Destructuring with Controller Request
// ============================================================================

class RequestDestructuringController extends Controller {
    fn extract_request_data(req: Any) -> Any {
        let {
            method,
            path,
            headers,
            params,
            query,
            json
        } = req;

        return {
            "status": 200,
            "body": json_stringify({
                "method": method,
                "path": path,
                "query_params": query,
                "has_json": json != null
            })
        };
    }

    fn extract_json_data(req: Any) -> Any {
        let { json: { name, email, preferences: { theme, notifications } } } = req;

        return {
            "status": 200,
            "body": json_stringify({
                "name": name,
                "email": email,
                "theme": theme,
                "notifications": notifications
            })
        };
    }

    fn extract_path_params(req: Any) -> Any {
        let { params: { user_id, post_id } } = req;

        return {
            "status": 200,
            "body": json_stringify({
                "user_id": user_id,
                "post_id": post_id
            })
        };
    }
}

// ============================================================================
// EXAMPLE 9: Destructuring with Database Results
// ============================================================================

class DatabaseDestructuringController extends Controller {
    static {
        this.db = solidb_connect("localhost", 6745, "api-key");
        this.database = "myapp";
    }

    fn process_user_result(req: Any) -> Any {
        let result = solidb_get(this.db, this.database, "users", "123");

        if (result == null) {
            return { "status": 404, "body": json_stringify({ "error": "User not found" }) };
        }

        let { _key: id, name, email, profile: { bio, avatar }, created_at } = result;

        return {
            "status": 200,
            "body": json_stringify({
                "id": id,
                "name": name,
                "email": email,
                "bio": bio,
                "avatar": avatar,
                "joined": created_at
            })
        };
    }

    fn process_query_results(req: Any) -> Any {
        let results = solidb_query(
            this.db,
            this.database,
            "FOR u IN users FILTER u.active == true RETURN u",
            {}
        );

        let active_users = results.map(fn({ _key: id, name, email }) {
            return { id, name, email };
        });

        return {
            "status": 200,
            "body": json_stringify({
                "count": len(active_users),
                "users": active_users
            })
        };
    }

    fn process_aggregation(req: Any) -> Any {
        let stats = solidb_query(
            this.db,
            this.database,
            `
                FOR o IN orders
                COLLECT status = o.status
                AGGREGATE total = SUM(o.amount), count = COUNT(o)
                RETURN { status, total, count }
            `,
            {}
        );

        let summary = stats.map(fn({ status, total, count }) {
            return {
                "status": status,
                "order_count": count,
                "total_amount": total
            };
        });

        return {
            "status": 200,
            "body": json_stringify({ "summary": summary })
        };
    }
}

// ============================================================================
// EXAMPLE 10: Renaming During Destructuring
// ============================================================================

class RenamingController extends Controller {
    fn rename_keys(req: Any) -> Any {
        let user = {
            "first_name": "John",
            "last_name": "Doe",
            "email_address": "john@example.com",
            "age_value": 30
        };

        let { first_name: firstName, last_name: lastName, email_address: email, age_value: age } = user;

        return {
            "status": 200,
            "body": json_stringify({
                "firstName": firstName,
                "lastName": lastName,
                "email": email,
                "age": age
            })
        };
    }

    fn api_response_mapping(req: Any) -> Any {
        let external_api_response = {
            "user_id": 12345,
            "display_name": "JohnDoe",
            "email_addr": "john@example.com",
            "account_created_ts": 1609459200,
            "last_login_ts": 1640995200
        };

        let {
            user_id: id,
            display_name: displayName,
            email_addr: email,
            account_created_ts: createdAt,
            last_login_ts: lastLoginAt
        } = external_api_response;

        return {
            "status": 200,
            "body": json_stringify({
                "id": id,
                "displayName": displayName,
                "email": email,
                "createdAt": createdAt,
                "lastLoginAt": lastLoginAt
            })
        };
    }
}

// ============================================================================
// EXAMPLE 11: Complex Real-World Examples
// ============================================================================

class ComplexController extends Controller {
    fn process_ecommerce_order(req: Any) -> Any {
        let order_data = {
            "order": {
                "id": "ORD-12345",
                "items": [
                    { "product_id": "P001", "name": "Laptop", "qty": 1, "price": 1000 },
                    { "product_id": "P002", "name": "Mouse", "qty": 2, "price": 25 }
                ],
                "shipping_address": {
                    "street": "123 Main St",
                    "city": "New York",
                    "zip": "10001"
                },
                "payment_info": {
                    "method": "credit_card",
                    "card_last4": "4242"
                }
            },
            "customer": {
                "id": "C001",
                "name": "John Doe",
                "email": "john@example.com"
            },
            "metadata": {
                "source": "web",
                "campaign": "spring_sale"
            }
        };

        let {
            order: {
                id: order_id,
                items,
                shipping_address: { city, zip },
                payment_info: { method, card_last4 }
            },
            customer: { name, email },
            metadata: { source, campaign }
        } = order_data;

        let subtotal = items.reduce(0, fn(sum, { price, qty }) {
            return sum + price * qty;
        });

        let shipping = city == "New York" ? 10 : 25;
        let total = subtotal + shipping;

        return {
            "status": 200,
            "body": json_stringify({
                "order_id": order_id,
                "customer": { name, email },
                "items_count": len(items),
                "subtotal": subtotal,
                "shipping": shipping,
                "total": total,
                "destination": { city, zip },
                "payment": { method, card_last4 },
                "source": source,
                "campaign": campaign
            })
        };
    }

    fn process_social_media_post(req: Any) -> Any {
        let post_data = {
            "id": "post_123",
            "author": {
                "id": "user_456",
                "username": "johndoe",
                "display_name": "John Doe",
                "avatar_url": "https://example.com/avatar.jpg"
            },
            "content": {
                "text": "Hello world!",
                "media": [
                    { "type": "image", "url": "https://example.com/img1.jpg" },
                    { "type": "video", "url": "https://example.com/vid1.mp4" }
                ]
            },
            "engagement": {
                "likes": 42,
                "shares": 10,
                "comments": 5
            },
            "timestamp": "2024-01-15T10:30:00Z"
        };

        let {
            id: post_id,
            author: { username, display_name: displayName, avatar_url: avatar },
            content: { text, media: [{ type: first_media_type, url: first_media_url }] },
            engagement: { likes, shares, comments },
            timestamp
        } = post_data;

        return {
            "status": 200,
            "body": json_stringify({
                "post_id": post_id,
                "author": { username, displayName, avatar },
                "text": text,
                "media_type": first_media_type,
                "media_url": first_media_url,
                "engagement": { likes, shares, comments },
                "posted_at": timestamp };
    }
}

//
            })
        ============================================================================
// COMPARISON: Before and After Destructuring
// ============================================================================

class ComparisonController extends Controller {
    fn without_destructuring(req: Any) -> Any {
        let user = req["json"]["user"];

        let name = user["name"];
        let email = user["email"];
        let profile = user["profile"];
        let bio = profile["bio"];
        let city = profile["location"]["city"];

        return {
            "status": 200,
            "body": json_stringify({
                "name": name,
                "email": email,
                "bio": bio,
                "city": city
            })
        };
    }

    fn with_destructuring(req: Any) -> Any {
        let { user: { name, email, profile: { bio, location: { city } } } } = req["json"];

        return {
            "status": 200,
            "body": json_stringify({
                "name": name,
                "email": email,
                "bio": bio,
                "city": city
            })
        };
    }
}

// ============================================================================
