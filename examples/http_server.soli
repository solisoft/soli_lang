// HTTP Server Example
// Demonstrates route-based HTTP server functionality

// Simple in-memory data store
let users = {};
let next_id = 1;

// Handler functions for each route
// Each handler takes a request Hash and returns a response Hash

fn handle_home(req: Any) -> Any {
    return {
        "status": 200,
        "headers": {"Content-Type": "text/plain"},
        "body": "Welcome to Solilang HTTP Server!"
    };
}

fn handle_health(req: Any) -> Any {
    return {
        "status": 200,
        "headers": {"Content-Type": "application/json"},
        "body": json_stringify({"status": "healthy"})
    };
}

fn handle_list_users(req: Any) -> Any {
    let user_list = values(users);
    return {
        "status": 200,
        "headers": {"Content-Type": "application/json"},
        "body": json_stringify(user_list)
    };
}

fn handle_get_user(req: Any) -> Any {
    let id = req["params"]["id"];

    if (has_key(users, id)) {
        return {
            "status": 200,
            "headers": {"Content-Type": "application/json"},
            "body": json_stringify(users[id])
        };
    } else {
        return {
            "status": 404,
            "headers": {"Content-Type": "application/json"},
            "body": json_stringify({"error": "User not found"})
        };
    }
}

fn handle_create_user(req: Any) -> Any {
    let data = json_parse(req["body"]);
    let id = str(next_id);
    next_id = next_id + 1;

    let user = {
        "id": id,
        "name": data["name"],
        "email": data["email"]
    };

    users[id] = user;

    return {
        "status": 201,
        "headers": {"Content-Type": "application/json"},
        "body": json_stringify(user)
    };
}

fn handle_update_user(req: Any) -> Any {
    let id = req["params"]["id"];

    if (has_key(users, id)) {
        let data = json_parse(req["body"]);
        let user = users[id];

        if (has_key(data, "name")) {
            user["name"] = data["name"];
        }
        if (has_key(data, "email")) {
            user["email"] = data["email"];
        }

        users[id] = user;

        return {
            "status": 200,
            "headers": {"Content-Type": "application/json"},
            "body": json_stringify(user)
        };
    } else {
        return {
            "status": 404,
            "headers": {"Content-Type": "application/json"},
            "body": json_stringify({"error": "User not found"})
        };
    }
}

fn handle_delete_user(req: Any) -> Any {
    let id = req["params"]["id"];

    if (has_key(users, id)) {
        let user = users[id];
        delete(users, id);

        return {
            "status": 200,
            "headers": {"Content-Type": "application/json"},
            "body": json_stringify({"deleted": user})
        };
    } else {
        return {
            "status": 404,
            "headers": {"Content-Type": "application/json"},
            "body": json_stringify({"error": "User not found"})
        };
    }
}

fn handle_echo(req: Any) -> Any {
    return {
        "status": 200,
        "headers": {"Content-Type": "application/json"},
        "body": json_stringify({
            "query": req["query"],
            "headers": req["headers"]
        })
    };
}

// Register routes
http_server_get("/", handle_home);
http_server_get("/health", handle_health);
http_server_get("/users", handle_list_users);
http_server_get("/users/:id", handle_get_user);
http_server_post("/users", handle_create_user);
http_server_put("/users/:id", handle_update_user);
http_server_delete("/users/:id", handle_delete_user);
http_server_get("/echo", handle_echo);

println("Starting HTTP server...");
println("Try these commands:");
println("  curl http://localhost:3000/");
println("  curl http://localhost:3000/health");
println("  curl http://localhost:3000/users");
println("  curl -X POST -H 'Content-Type: application/json' -d '{\"name\":\"Alice\",\"email\":\"alice@example.com\"}' http://localhost:3000/users");
println("  curl http://localhost:3000/users/1");
println("  curl 'http://localhost:3000/echo?foo=bar&baz=qux'");
println("");

// Start the server (blocking)
http_server_listen(3000);
