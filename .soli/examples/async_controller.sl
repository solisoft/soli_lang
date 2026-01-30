// ============================================================================
// Async Controller Examples for AI/LLM Code Generation
// ============================================================================
//
// AI AGENT GUIDE:
// ---------------
// This file demonstrates async/await patterns in Soli controllers.
// Use async functions for I/O operations like HTTP requests, database queries,
// file operations, and any operation that returns a Promise.
//
// ASYNC/AWAIT SYNTAX:
// - Mark functions with 'async' keyword: async fn function_name(req: Any) -> Any
// - Use 'await' to wait for Promises: let data = await http_get(url)
// - Return Promises for async operations: return promise(value)
// - Use promise_all() for parallel execution: await promise_all([p1, p2])
//
// ERROR HANDLING:
// - Use try/catch in async functions for error handling
// - Throw errors with: throw Error.new("message", { code: 500 })
// - Catch specific error types: catch (e: ValueError) { ... }
//
// ============================================================================

// ============================================================================
// EXAMPLE 1: Basic Async Controller with HTTP Requests
// ============================================================================

class ApiController extends Controller {
    static {
        this.layout = null;
    }

    async fn fetch_external_data(req: Any) -> Any {
        let base_url = "https://api.example.com";

        try {
            let response = await http_get(base_url + "/data");

            if (response["status"] != 200) {
                throw Error.new("External API error", { status: response["status"] });
            }

            return {
                "status": 200,
                "body": json_stringify({
                    "success": true,
                    "data": response["body"]
                })
            };
        } catch (e) {
            print("[ERROR] Failed to fetch external data:", e);

            return {
                "status": 502,
                "body": json_stringify({
                    "error": "Failed to fetch data",
                    "message": e["message"]
                })
            };
        }
    }

    async fn fetch_multiple_apis(req: Any) -> Any {
        let urls = [
            "https://api.service1.com/data",
            "https://api.service2.com/data",
            "https://api.service3.com/data"
        ];

        try {
            let promises = urls.map(fn(url) {
                return http_get(url);
            });

            let results = await promise_all(promises);

            let successful = results.filter(fn(r) { return r["status"] == 200; });
            let failed = results.filter(fn(r) { return r["status"] != 200; });

            return {
                "status": 200,
                "body": json_stringify({
                    "success": true,
                    "successful_count": len(successful),
                    "failed_count": len(failed),
                    "results": successful.map(fn(r) { return json_parse(r["body"]); })
                })
            };
        } catch (e) {
            return {
                "status": 500,
                "body": json_stringify({
                    "error": "Failed to fetch all APIs",
                    "message": e["message"]
                })
            };
        }
    }
}

// ============================================================================
// EXAMPLE 2: Async Controller with SolidB Database Operations
// ============================================================================

class DataSyncController extends Controller {
    static {
        this.db = solidb_connect("localhost", 6745, "api-key = "my");
        this.databaseapp";
    }

    async fn sync_users(req: Any) -> Any {
        try {
            let external_users = await http_get("https://legacy-system.com/users");

            if (external_users["status"] != 200) {
                throw Error.new("Legacy system unavailable", { code: 503 });
            }

            let users = json_parse(external_users["body"]);
            let sync_count = 0;
            let errors = [];

            for user in users {
                try {
                    let result = await solidb_query(
                        this.db,
                        this.database,
                        "UPSERT { _key: @key } INSERT @doc UPDATE @doc IN users RETURN NEW",
                        { "key": user["id"], "doc": user }
                    );
                    sync_count = sync_count + 1;
                } catch (e) {
                    errors.push({
                        "user_id": user["id"],
                        "error": e["message"]
                    });
                }
            }

            return {
                "status": 200,
                "body": json_stringify({
                    "success": true,
                    "synced": sync_count,
                    "errors": errors
                })
            };
        } catch (e) {
            return {
                "status": 500,
                "body": json_stringify({
                    "error": "Sync failed",
                    "message": e["message"]
                })
            };
        }
    }

    async fn batch_create_posts(req: Any) -> Any {
        let data = req["json"];
        let posts = data["posts"];

        if (posts == null || len(posts) == 0) {
            throw ValueError.new("No posts provided");
        }

        try {
            let promises = posts.map(fn(post) {
                return solidb_insert(this.db, this.database, "posts", {
                    "title": post["title"],
                    "content": post["content"],
                    "author": post["author"],
                    "created_at": DateTime.now()
                });
            });

            let results = await promise_all(promises);

            return {
                "status": 201,
                "body": json_stringify({
                    "success": true,
                    "created": len(results),
                    "posts": results
                })
            };
        } catch (e) {
            if (e["type"] == "ValueError") {
                return {
                    "status": 400,
                    "body": json_stringify({
                        "error": "Validation error",
                        "message": e["message"]
                    })
                };
            }

            return {
                "status": 500,
                "body": json_stringify({
                    "error": "Batch creation failed",
                    "message": e["message"]
                })
            };
        }
    }
}

// ============================================================================
// EXAMPLE 3: Async Controller with File Operations
// ============================================================================

class FileController extends Controller {
    async fn process_upload(req: Any) -> Any {
        let file = req["file"];

        if (file == null) {
            throw ValueError.new("No file uploaded");
        }

        try {
            let content = await file_read(file["path"]);

            let processed = await process_content(content);

            let save_path = "/uploads/processed/" + file["filename"];
            await file_write(save_path, processed);

            return {
                "status": 200,
                "body": json_stringify({
                    "success": true,
                    "path": save_path,
                    "size": len(processed)
                })
            };
        } catch (e: ValueError) {
            return {
                "status": 400,
                "body": json_stringify({
                    "error": "Validation error",
                    "message": e["message"]
                })
            };
        } catch (e) {
            print("[ERROR] File processing failed:", e);

            return {
                "status": 500,
                "body": json_stringify({
                    "error": "Processing failed",
                    "message": "An error occurred while processing the file"
                })
            };
        } finally {
            if (file != null && has_key(file, "temp_path")) {
                file_delete(file["temp_path"]);
            }
        }
    }

    async fn import_csv_data(req: Any) -> Any {
        let file_path = req["upload_path"];

        try {
            let content = await file_read(file_path);
            let lines = content.split("\n");
            let records = [];

            for line in lines {
                if (line.trim() == "") { continue; }

                let fields = line.split(",");
                records.push({
                    "name": fields[0],
                    "email": fields[1],
                    "imported_at": DateTime.now()
                });
            }

            let result = await solidb_insert(this.db, this.database, "imports", {
                "record_count": len(records),
                "records": records
            });

            return {
                "status": 200,
                "body": json_stringify({
                    "success": true,
                    "imported": len(records),
                    "import_id": result["_key"]
                })
            };
        } catch (e) {
            return {
                "status": 500,
                "body": json_stringify({
                    "error": "Import failed",
                    "message": e["message"]
                })
            };
        }
    }
}

// ============================================================================
// EXAMPLE 4: Async Middleware for Authentication
// ============================================================================

class AuthController extends Controller {
    async fn protected_resource(req: Any) -> Any {
        let token = req["headers"]["Authorization"];

        if (token == null) {
            throw Error.new("Missing authorization token", { code: 401 });
        }

        try {
            let user_data = await verify_token(token);

            if (user_data == null) {
                throw Error.new("Invalid token", { code: 401 });
            }

            return {
                "status": 200,
                "body": json_stringify({
                    "user": user_data,
                    "message": "Access granted"
                })
            };
        } catch (e) {
            return {
                "status": e["code"] ?? 500,
                "body": json_stringify({
                    "error": e["message"]
                })
            };
        }
    }

    async fn refresh_session(req: Any) -> Any {
        let refresh_token = req["json"]["refresh_token"];

        try {
            let new_token = await generate_session(refresh_token);

            return {
                "status": 200,
                "body": json_stringify({
                    "success": true,
                    "token": new_token
                })
            };
        } catch (e) {
            return {
                "status": 401,
                "body": json_stringify({
                    "error": "Invalid refresh token"
                })
            };
        }
    }
}

// ============================================================================
// EXAMPLE 5: Async Background Jobs
// ============================================================================

class JobController extends Controller {
    async fn enqueue_job(req: Any) -> Any {
        let job_type = req["json"]["type"];
        let job_data = req["json"]["data"] ?? {};

        let job = await redis_queue_push("jobs", {
            "type": job_type,
            "data": job_data,
            "created_at": DateTime.now()
        });

        return {
            "status": 202,
            "body": json_stringify({
                "success": true,
                "job_id": job["id"],
                "status": "queued"
            })
        };
    }

    async fn process_job_queue(req: Any) -> Any {
        let processed = 0;
        let errors = [];

        while (true) {
            let job = await redis_queue_pop("jobs");

            if (job == null) { break; }

            try {
                await execute_job(job);
                processed = processed + 1;
            } catch (e) {
                errors.push({
                    "job_id": job["id"],
                    "error": e["message"]
                });
            }
        }

        return {
            "status": 200,
            "body": json_stringify({
                "processed": processed,
                "errors": errors
            })
        };
    }
}

// ============================================================================
// EXAMPLE 6: Async Controller with WebSocket Events
// ============================================================================

class RealtimeController extends Controller {
    async fn broadcast_update(req: Any) -> Any {
        let event = req["json"]["event"];
        let data = req["json"]["data"];

        try {
            await redis_publish("events:" + event, data);

            return {
                "status": 200,
                "body": json_stringify({
                    "success": true,
                    "event": event,
                    "broadcasted": true
                })
            };
        } catch (e) {
            return {
                "status": 500,
                "body": json_stringify({
                    "error": "Broadcast failed",
                    "message": e["message"]
                })
            };
        }
    }

    async fn subscribe_channel(req: Any) -> Any {
        let channel = req["params"]["channel"];

        let subscription = await redis_subscribe(channel, fn(message) {
            broadcast_to_websocket(channel, message);
        });

        return {
            "status": 200,
            "body": json_stringify({
                "success": true,
                "channel": channel,
                "subscribed": true
            })
        };
    }
}

// ============================================================================
// PROMISE HELPER FUNCTIONS
// ============================================================================

fn delay(ms: Int) -> Any {
    return promise(fn(resolve) {
        set_timeout(fn() {
            resolve({"completed": true, "ms": ms});
        }, ms);
    });
}

fn retry_async(fn_to_retry: Fn, max_attempts: Int, delay_ms: Int) -> Any {
    return promise(fn(resolve) {
        let attempts = 0;
        let last_error = null;

        fn attempt() {
            attempts = attempts + 1;
            try {
                let result = fn_to_retry();
                resolve(result);
            } catch (e) {
                last_error = e;
                if (attempts < max_attempts) {
                    set_timeout(attempt, delay_ms);
                } else {
                    throw last_error;
                }
            }
        }

        attempt();
    });
}

fn timeout_async(promise: Any, timeout_ms: Int) -> Any {
    return promise(fn(resolve) {
        let completed = false;
        let timeout_id = null;

        promise.then(fn(result) {
            if (!completed) {
                completed = true;
                resolve(result);
            }
        });

        timeout_id = set_timeout(fn() {
            if (!completed) {
                completed = true;
                throw Error.new("Operation timed out", { ms: timeout_ms });
            }
        }, timeout_ms);
    });
}

// ============================================================================
// COMPLETE EXAMPLE: Async Controller with All Patterns
// ============================================================================

class CompleteAsyncController extends Controller {
    static {
        this.layout = null;
        this.timeout = 30000;
    }

    async fn comprehensive_example(req: Any) -> Any {
        try {
            let { user_id, action } = req["params"];

            if (user_id == null) {
                throw ValueError.new("user_id is required");
            }

            let user = await solidb_get(this.db, this.database, "users", user_id);

            if (user == null) {
                throw KeyError.new("User not found", { user_id: user_id });
            }

            let profile = await timeout_async(
                fetch_user_profile(user_id),
                this.timeout
            );

            let activities = await solidb_query(
                this.db,
                this.database,
                "FOR a IN activities FILTER a.user_id == @user_id SORT a.created_at DESC LIMIT 10 RETURN a",
                { "user_id": user_id }
            );

            let notifications = await fetch_notifications(user_id);

            return {
                "status": 200,
                "body": json_stringify({
                    "success": true,
                    "user": {
                        "id": user["_key"],
                        "name": user["name"],
                        "email": user["email"]
                    },
                    "profile": profile,
                    "activities": activities,
                    "notifications": notifications,
                    "retrieved_at": DateTime.now()
                })
            };
        } catch (e: ValueError) {
            return {
                "status": 400,
                "body": json_stringify({
                    "error": "Validation error",
                    "message": e["message"]
                })
            };
        } catch (e: KeyError) {
            return {
                "status": 404,
                "body": json_stringify({
                    "error": "Not found",
                    "message": e["message"]
                })
            };
        } catch (e: Error) {
            if (e["code"] == 504) {
                return {
                    "status": 504,
                    "body": json_stringify({
                        "error": "Timeout",
                        "message": "The request took too long to process"
                    })
                };
            }

            return {
                "status": 500,
                "body": json_stringify({
                    "error": "Internal server error",
                    "message": "An unexpected error occurred"
                })
            };
        } finally {
            cleanup_resources();
        }
    }
}

// ============================================================================
// HELPER FUNCTIONS (would normally be in separate modules)
// ============================================================================

async fn fetch_user_profile(user_id: String) -> Any {
    let response = await http_get("https://api.example.com/users/" + user_id + "/profile");
    return json_parse(response["body"]);
}

async fn fetch_notifications(user_id: String) -> Any {
    let cache_key = "notifications:" + user_id;
    let cached = await redis_get(cache_key);

    if (cached != null) {
        return json_parse(cached);
    }

    let notifications = await solidb_query(
        solidb_connect("localhost", 6745, ""),
        "myapp",
        "FOR n IN notifications FILTER n.user_id == @user_id SORT n.created_at DESC LIMIT 20 RETURN n",
        { "user_id": user_id }
    );

    await redis_setex(cache_key, 300, json_stringify(notifications));

    return notifications;
}

fn cleanup_resources() -> Any {
    print("[CLEANUP] Releasing resources...");
}

// ============================================================================
