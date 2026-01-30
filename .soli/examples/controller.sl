// ============================================================================
// PostsController - Example CRUD Controller for AI/LLM Code Generation
// ============================================================================
//
// AI AGENT GUIDE:
// ---------------
// This file demonstrates the standard Soli MVC controller conventions.
// Use this as a template when generating new controllers.
//
// KEY CONVENTIONS:
// 1. Class name: PascalCase ending with "Controller"
// 2. File name: snake_case ending with "_controller.sl"
// 3. Base class: All controllers extend "Controller"
// 4. Method signature: fn method_name(req: Any) -> Any
// 5. Response types: render() for HTML, dict for JSON/redirect
//
// TEMPLATE FOR AI GENERATION:
// ---------------------------
// class {Resource}Controller extends Controller {
//     static {
//         this.layout = "application";
//         this.before_action = fn(req) { /* auth check */ };
//     }
//     
//     fn index(req: Any) -> Any { /* list resources */ }
//     fn show(req: Any) -> Any { /* show single resource */ }
//     fn new(req: Any) -> Any { /* show creation form */ }
//     fn create(req: Any) -> Any { /* handle creation */ }
//     fn edit(req: Any) -> Any { /* show edit form */ }
//     fn update(req: Any) -> Any { /* handle update */ }
//     fn destroy(req: Any) -> Any { /* handle deletion */ }
// }
//
// ROUTE MAPPINGS:
// ---------------
// GET    /posts              → index
// GET    /posts/:id          → show
// GET    /posts/new          → new (form)
// POST   /posts              → create
// GET    /posts/:id/edit     → edit (form)
// PUT    /posts/:id          → update
// DELETE /posts/:id          → destroy
//
// ============================================================================

class PostsController extends Controller {
    // STATIC BLOCK - Controller-wide configuration
    // ------------------------------------------------
    // - Layout: Which layout template to use (optional, defaults to "application")
    // - before_action: Callback before each action (e.g., authentication)
    // - after_action: Callback after each action (e.g., logging)
    static {
        this.layout = "application";
        
        // Run _authenticate before show, edit, update, destroy actions
        this.before_action = fn(req) {
            let action = req["action"];
            if (action == "show" || action == "edit" || action == "update" || action == "destroy") {
                return _authenticate(req);
            }
            return {"continue": true, "request": req};
        };
    }

    // INDEX ACTION - List all posts
    // Usage: GET /posts
    // ------------------------------------------------
    fn index(req: Any) -> Any {
        // In real app: fetch from database using Post.all()
        let posts = [
            {"id": 1, "title": "First Post", "content": "Hello World"},
            {"id": 2, "title": "Second Post", "content": "Soli MVC is great!"}
        ];
        
        // render(template_path, data_hash) → HTML response
        // template_path: "controller_name/action_name" (without .sl)
        // data_hash: variables passed to template, accessed as @variable_name
        return render("posts/index", {
            "posts": posts,
            "title": "All Posts"
        });
    }

    // SHOW ACTION - Display single post
    // Usage: GET /posts/:id
    // ------------------------------------------------
    fn show(req: Any) -> Any {
        // Access path parameters via req["params"]["param_name"]
        let id = req["params"]["id"];
        
        // In real app: Post.find(id)
        let post = {"id": id, "title": "Post " + id, "content": "Content here"};
        
        if (post == null) {
            // Return 404 response
            return {
                "status": 404,
                "body": "Post not found"
            };
        }
        
        return render("posts/show", {
            "post": post,
            "title": post["title"]
        });
    }

    // NEW ACTION - Show creation form
    // Usage: GET /posts/new
    // ------------------------------------------------
    fn new(req: Any) -> Any {
        return render("posts/new", {
            "post": {"title": "", "content": ""},
            "title": "New Post"
        });
    }

    // CREATE ACTION - Handle form submission
    // Usage: POST /posts
    // ------------------------------------------------
    fn create(req: Any) -> Any {
        // Access JSON body via req["json"]
        let data = req["json"];
        
        // In real app: validate and save to database
        // let result = Post.create(data);
        let new_id = 3;  // Generated ID
        
        // Redirect after successful creation
        // Return redirect URL to redirect client
        return {
            "status": 302,
            "headers": {"Location": "/posts/" + new_id}
        };
    }

    // EDIT ACTION - Show edit form
    // Usage: GET /posts/:id/edit
    // ------------------------------------------------
    fn edit(req: Any) -> Any {
        let id = req["params"]["id"];
        
        // In real app: Post.find(id)
        let post = {"id": id, "title": "Post " + id, "content": "Content here"};
        
        if (post == null) {
            return {"status": 404, "body": "Post not found"};
        }
        
        return render("posts/edit", {
            "post": post,
            "title": "Edit " + post["title"]
        });
    }

    // UPDATE ACTION - Handle edit form submission
    // Usage: PUT /posts/:id
    // ------------------------------------------------
    fn update(req: Any) -> Any {
        let id = req["params"]["id"];
        let data = req["json"];
        
        // In real app: Post.update(id, data)
        let success = true;
        
        if (success) {
            return {
                "status": 302,
                "headers": {"Location": "/posts/" + id}
            };
        }
        
        // Return errors
        return {
            "status": 422,
            "body": json_stringify({"errors": {"title": "Title is required"}})
        };
    }

    // DESTROY ACTION - Delete a post
    // Usage: DELETE /posts/:id
    // ------------------------------------------------
    fn destroy(req: Any) -> Any {
        let id = req["params"]["id"];
        
        // In real app: Post.destroy(id)
        Post.destroy(id);
        
        return {
            "status": 302,
            "headers": {"Location": "/posts"}
        };
    }

    // PRIVATE METHODS - Helper functions (prefixed with _)
    // ------------------------------------------------
    // Private methods are implementation details not exposed as actions.
    // They can be called from other methods in the class.
    
    fn _authenticate(req: Any) -> Any {
        // Example: Check session for authentication
        let authenticated = session_get("authenticated");
        
        if (authenticated != true) {
            // Return redirect to login
            return {
                "continue": false,
                "response": {
                    "status": 302,
                    "headers": {"Location": "/users/login"}
                }
            };
        }
        
        return {"continue": true, "request": req};
    }

    fn _build_post_params(req: Any) -> Any {
        // Extract and sanitize post parameters from request
        let data = req["json"];
        return {
            "title": data["title"],
            "content": data["content"],
            "author_id": session_get("user_id")
        };
    }
}

// ============================================================================
// SOLIDB DATABASE INTEGRATION EXAMPLES
// ============================================================================
//
// AI AGENT GUIDE - SOLIDB:
// ------------------------
// SoliDB (SolidB) is a multi-document database with SDBQL query language.
// See: https://github.com/solisoft/solidb
//
// CONNECTION:
// - Default host: localhost:6745
// - Authentication: Bearer token or X-API-Key header
//
// SOLIDB BUILT-IN FUNCTIONS IN SOLI:
// - solidb_connect(host, port, api_key) → connection
// - solidb_query(db, database, query, params) → result array
// - solidb_insert(db, database, collection, document) → document with _id
// - solidb_get(db, database, collection, id) → document or null
// - solidb_update(db, database, collection, id, data) → updated document
// - solidb_delete(db, database, collection, id) → success boolean
// - solidb_transaction(db, database, fn(tx) { ... }) → transaction result
//
// SDBQL QUERY SYNTAX:
// FOR doc IN collection FILTER condition RETURN doc
//
// ============================================================================

// SOLIDB INTEGRATION IN CONTROLLERS
// ==================================

class PostsController extends Controller {
    static {
        this.layout = "application";
        
        // Initialize SolidB connection (shared across requests)
        this.db = solidb_connect("localhost", 6745, "your-api-key");
        this.database = "myapp";
    }
    
    // INDEX - List all posts with SolidB
    // SDBQL: FOR doc IN posts RETURN doc
    fn index(req: Any) -> Any {
        let query = "FOR doc IN posts SORT doc.created_at DESC RETURN doc";
        let posts = solidb_query(this.db, this.database, query, {});
        
        return render("posts/index", {
            "posts": posts,
            "title": "All Posts"
        });
    }
    
    // SHOW - Display single post
    // solidb_get returns document by ID
    fn show(req: Any) -> Any {
        let id = req["params"]["id"];
        let post = solidb_get(this.db, this.database, "posts", id);
        
        if (post == null) {
            return {"status": 404, "body": "Post not found"};
        }
        
        return render("posts/show", {
            "post": post,
            "title": post["title"]
        });
    }
    
    // CREATE - Insert new post
    // solidb_insert returns document with _id and _key
    fn create(req: Any) -> Any {
        let data = req["json"];
        
        let document = {
            "title": data["title"],
            "content": data["content"],
            "author": data["author"],
            "created_at": DateTime.now(),
            "status": "published"
        };
        
        let result = solidb_insert(this.db, this.database, "posts", document);
        
        return {
            "status": 201,
            "body": json_stringify({
                "success": true,
                "post": result
            })
        };
    }
    
    // UPDATE - Modify existing post
    fn update(req: Any) -> Any {
        let id = req["params"]["id"];
        let data = req["json"];
        
        let updates = {
            "title": data["title"],
            "content": data["content"],
            "updated_at": DateTime.now()
        };
        
        let result = solidb_update(this.db, this.database, "posts", id, updates);
        
        return {
            "status": 200,
            "body": json_stringify({
                "success": true,
                "post": result
            })
        };
    }
    
    // DESTROY - Delete post
    fn destroy(req: Any) -> Any {
        let id = req["params"]["id"];
        let success = solidb_delete(this.db, this.database, "posts", id);
        
        if (success) {
            return {
                "status": 302,
                "headers": {"Location": "/posts"}
            };
        }
        
        return {"status": 404, "body": "Post not found"};
    }
    
    // ADVANCED QUERY - Filtered search
    // SDBQL with FILTER and parameters
    fn search(req: Any) -> Any {
        let query_params = req["query"];
        let search_term = query_params["q"];
        let status = query_params["status"] ?? "published";
        
        // Parameterized SDBQL query
        let query = `
            FOR doc IN posts
            FILTER doc.status == @status
            FILTER LIKE(doc.title, @search_term, true)
            SORT doc.created_at DESC
            LIMIT 20
            RETURN doc
        `;
        
        let params = {
            "status": status,
            "search_term": "%" + search_term + "%"
        };
        
        let results = solidb_query(this.db, this.database, query, params);
        
        return {
            "status": 200,
            "body": json_stringify({
                "query": search_term,
                "count": len(results),
                "results": results
            })
        };
    }
    
    // TRANSACTION - Atomic operations
    fn transfer_post(req: Any) -> Any {
        let data = req["json"];
        let post_id = data["post_id"];
        let new_author = data["new_author"];
        
        // Atomic transaction
        let result = solidb_transaction(this.db, this.database, fn(tx) {
            // Get original post
            let post = solidb_get(tx, this.database, "posts", post_id);
            if (post == null) {
                throw "Post not found";
            }
            
            // Update post author
            solidb_update(tx, this.database, "posts", post_id, {
                "author": new_author,
                "transferred_at": DateTime.now()
            });
            
            // Log transfer
            solidb_insert(tx, this.database, "post_transfers", {
                "post_id": post_id,
                "from_author": post["author"],
                "to_author": new_author,
                "transferred_at": DateTime.now()
            });
            
            return {"success": true, "post_id": post_id};
        });
        
        return {
            "status": 200,
            "body": json_stringify(result)
        };
    }
}

// ============================================================================
// SDBQL QUERY EXAMPLES
// ============================================================================
//
// BASIC QUERIES:
// FOR doc IN posts RETURN doc                           // All documents
// FOR doc IN posts FILTER doc.status == "published" RETURN doc  // Filtered
// FOR doc IN posts SORT doc.created_at DESC LIMIT 10 RETURN doc  // Sorted & limited
//
// AGGREGATION:
// FOR doc IN posts
//   COLLECT status = doc.status WITH COUNT INTO count
//   RETURN {status, count}
//
// JOINS:
// FOR post IN posts
//   FOR author IN authors
//     FILTER post.author_id == author._key
//     RETURN {post, author}
//
// UPSERT:
// UPSERT { _key: @key }
//   INSERT { _key: @key, count: 1 }
//   UPDATE { count: OLD.count + 1 }
//   IN collection
//
// ============================================================================

// ============================================================================
// LUA SCRIPTING IN SOLIDB
// ============================================================================
//
// Lua scripts can be defined in SoliDB for custom endpoints:
//
// -- In SoliDB _scripts collection
// function handle(request)
//     local db = DB.connect()
//     local posts = db.posts:query("FOR p IN posts RETURN p", {})
//     return {
//         status = 200,
//         body = json.encode(posts)
//     }
// end
//
// Available in Lua:
// - db:query(sdbql, params)
// - db.collection:insert(doc)
// - db.collection:get(id)
// - db.collection:update(id, data)
// - db.collection:delete(id)
// - db:transaction(fn)
// - http.request(url, options)
//
// ============================================================================

// ============================================================================
// SDBQL COMPLETE FUNCTION REFERENCE AND EXAMPLES
// ============================================================================
//
// This section documents all SDBQL functions with examples for AI agents.
//
// SDBQL FUNCTION CATEGORIES:
// 1. String Functions
// 2. Numeric/Math Functions
// 3. Array Functions
// 4. DateTime Functions
// 5. Type Conversion Functions
// 6. Aggregate Functions
// 7. Geo Functions
// 8. Vector Functions
// 9. Phonetic Functions
// 10. JSON Functions
// 11. Crypto Functions
//
// ============================================================================

// ============================================================================
// 1. STRING FUNCTIONS
// ============================================================================
//
// SDBQL Examples:
//
// RETURN UPPER(doc.title)
// RETURN LOWER(doc.email)
// RETURN CONCAT(doc.first_name, " ", doc.last_name)
// RETURN SUBSTRING(doc.description, 0, 100)
// RETURN TRIM(doc.content)
// RETURN REPLACE(doc.text, "old", "new")
// RETURN SPLIT(doc.tags, ",")
// RETURN CONTAINS(doc.content, "keyword")
// RETURN STARTS_WITH(doc.url, "https://")
// RETURN ENDS_WITH(doc.filename, ".pdf")
// RETURN LENGTH(doc.content)
// RETURN LEFT(doc.name, 50)
// RETURN RIGHT(doc.name, 10)
// RETURN REVERSE(doc.code)
//
// ============================================================================

// ============================================================================
// 2. NUMERIC/MATH FUNCTIONS
// ============================================================================
//
// SDBQL Examples:
//
// RETURN TO_NUMBER(doc.price)
// RETURN FLOOR(doc.average)
// RETURN CEIL(doc.score)
// RETURN ROUND(doc.rating)
// RETURN ABS(doc.delta)
// RETURN SQRT(doc.value)
// RETURN POWER(doc.base, 2)
// RETURN MOD(doc.value, 10)
// RETURN MIN(doc.values)
// RETURN MAX(doc.values)
// RETURN SUM(doc.prices)
// RETURN AVG(doc.scores)
//
// ============================================================================

// ============================================================================
// 3. ARRAY FUNCTIONS
// ============================================================================
//
// SDBQL Examples:
//
// RETURN FIRST(doc.items)
// RETURN LAST(doc.items)
// RETURN PUSH(doc.tags, "new_tag")
// RETURN POP(doc.items)
// RETURN APPEND(doc.list1, doc.list2)
// RETURN UNIQUE(doc.duplicates)
// RETURN SORTED(doc.numbers)
// RETURN SORTED_DESC(doc.numbers)
// RETURN REVERSE(doc.array)
// RETURN FLATTEN(doc.nested)
// RETURN SLICE(doc.items, 0, 10)
// RETURN POSITION(doc.items, "value")
// RETURN REMOVE_VALUE(doc.items, "old")
// RETURN REMOVE_NTH(doc.items, 5)
// RETURN LENGTH(doc.items)
//
// ============================================================================

// ============================================================================
// 4. DATETIME FUNCTIONS
// ============================================================================
//
// SDBQL Examples:
//
// RETURN DATE_FORMAT(doc.created_at, "%Y-%m-%d")
// RETURN DATE_NOW()
// RETURN DATE_ADD(doc.date, 7, "day")
// RETURN DATE_SUB(doc.date, 1, "month")
// RETURN DATE_DIFF(doc.end, doc.start, "day")
// RETURN IS_SAME_DATE(doc.a, doc.b)
// RETURN IS_BEFORE(doc.a, doc.b)
// RETURN IS_AFTER(doc.a, doc.b)
//
// ============================================================================

// ============================================================================
// 5. TYPE CONVERSION FUNCTIONS
// ============================================================================
//
// SDBQL Examples:
//
// RETURN TO_STRING(doc.value)
// RETURN TO_NUMBER(doc.string)
// RETURN TO_BOOL(doc.value)
// RETURN TO_ARRAY(doc.value)
// RETURN IS_NULL(doc.value)
// RETURN IS_BOOL(doc.value)
// RETURN IS_NUMBER(doc.value)
// RETURN IS_STRING(doc.value)
// RETURN IS_INTEGER(doc.value)
// RETURN IS_ARRAY(doc.value)
// RETURN IS_OBJECT(doc.value)
// RETURN IS_DATETIME(doc.value)
//
// ============================================================================

// ============================================================================
// 6. AGGREGATE FUNCTIONS (COLLECT)
// ============================================================================
//
// SDBQL Examples:
//
// FOR doc IN orders
//   COLLECT status = doc.status WITH COUNT INTO count
//   RETURN {status, count}
//
// FOR doc IN sales
//   COLLECT year = DATE_FORMAT(doc.date, "%Y")
//   AGGREGATE total = SUM(doc.amount), avg = AVG(doc.amount)
//   RETURN {year, total, avg}
//
// FOR doc IN products
//   COLLECT category = doc.category
//   AGGREGATE min_price = MIN(doc.price), max_price = MAX(doc.price)
//   RETURN {category, min_price, max_price}
//
// FOR doc IN events
//   COLLECT BY doc.user_id
//   AGGREGATE event_count = COUNT(doc)
//   RETURN {user_id: doc.user_id, event_count}
//
// ============================================================================

// ============================================================================
// 7. GEO FUNCTIONS
// ============================================================================
//
// SDBQL Examples:
//
// RETURN DISTANCE(doc.location, [-122.4194, 37.7749])
// RETURN GEO_DISTANCE(doc.from, doc.to)
// RETURN GEO_CONTAINS(doc.area, doc.point)
// RETURN GEO_INTERSECTS(doc.a, doc.b)
//
// Geo Index Query:
// FOR doc IN locations
//   FILTER GEO_DISTANCE(doc.coords, @center) < @radius
//   SORT GEO_DISTANCE(doc.coords, @center)
//   RETURN doc
//
// ============================================================================

// ============================================================================
// 8. VECTOR FUNCTIONS
// ============================================================================
//
// SDBQL Examples:
//
// RETURN VECTOR_SIMILARITY(doc.embedding, @query_vector)
// RETURN VECTOR_COSINE(doc.vectors)
// RETURN VECTOR_EUCLIDEAN(doc.a, doc.b)
// RETURN VECTOR_NORMALIZE(doc.vector)
//
// Vector Search:
// FOR doc IN products
//   LET score = VECTOR_SIMILARITY(doc.embedding, @query)
//   FILTER score > @threshold
//   SORT score DESC
//   RETURN {doc, score}
//
// ============================================================================

// ============================================================================
// 9. PHONETIC FUNCTIONS
// ============================================================================
//
// SDBQL Examples:
//
// RETURN SOUNDEX(doc.name)
// RETURN METAPHONE(doc.word)
// RETURN DOUBLE_METAPHONE(doc.word)
// RETURN NYSIIS(doc.name)
// RETURN CAVERPHONE(doc.name)
// RETURN SOUNDEX_FR(doc.name)    -- French
// RETURN SOUNDEX_ES(doc.name)    -- Spanish
// RETURN SOUNDEX_IT(doc.name)    -- Italian
// RETURN COLOGNE(doc.name)       -- German
//
// ============================================================================

// ============================================================================
// 10. JSON FUNCTIONS
// ============================================================================
//
// SDBQL Examples:
//
// RETURN JSON_PARSE(doc.json_string)
// RETURN JSON_STRINGIFY(doc.object)
// RETURN JSON_VALUE(doc.json, "$.path.to.value")
// RETURN JSON_QUERY(doc.json, "$.array")
// RETURN JSON_SET(doc.json, "$.key", "value")
// RETURN JSON_REMOVE(doc.json, "$.old_key")
// RETURN JSON_INSERT(doc.json, "$.new_key", "value")
// RETURN JSON_REPLACE(doc.json, "$.key", "new_value")
//
// ============================================================================

// ============================================================================
// 11. CRYPTO FUNCTIONS
// ============================================================================
//
// SDBQL Examples:
//
// RETURN MD5(doc.content)
// RETURN SHA256(doc.data)
// RETURN SHA512(doc.data)
// RETURN HMAC_SHA256(doc.message, doc.secret)
// RETURN BASE64_ENCODE(doc.data)
// RETURN BASE64_DECODE(doc.encoded)
// RETURN CRYPTO_RANDOM()
// RETURN CRYPTO_RANDOM_RANGE(1, 100)
//
// ============================================================================

// ============================================================================
// COMPLETE SDBQL QUERY EXAMPLES
// ============================================================================
//
// BASIC QUERIES:
// FOR doc IN users RETURN doc
// FOR doc IN users FILTER doc.active == true RETURN doc
// FOR doc IN users FILTER doc.age >= 18 SORT doc.name LIMIT 10 RETURN doc
//
// PARAMETERIZED QUERIES:
// FOR doc IN users FILTER doc.status == @status RETURN doc
// FOR doc IN products FILTER doc.price BETWEEN @min AND @max RETURN doc
//
// JOINS:
// FOR user IN users
//   FOR post IN posts
//     FILTER post.user_id == user._key
//     RETURN {user, post}
//
// FOR order IN orders
//   FOR item IN order.items
//     FOR product IN products
//       FILTER item.product_id == product._key
//       RETURN {order, item, product}
//
// UPSERT:
// UPSERT { _key: @key }
//   INSERT { _key: @key, count: 1 }
//   UPDATE { count: OLD.count + 1 }
//   IN page_views
//
// GRAPH TRAVERSAL:
// FOR v, e, p IN 1..3 ANY @start_vertex GRAPH "my_graph"
//   RETURN {vertex: v, edge: e, path: p}
//
// FULLTEXT SEARCH:
// FOR doc IN users
//   SEARCH ANALYZER(doc.bio IN TEXT("developer"), "text_en")
//   RETURN doc
//
// SUBQUERIES:
// FOR user IN users
//   LET posts = (FOR p IN posts FILTER p.user_id == user._key RETURN p)
//   RETURN {user, posts}
//
// WINDOW FUNCTIONS:
// FOR doc IN sales
//   SORT doc.date
//   LET running_total = SUM(doc.amount) OVER (ORDER BY doc.date ROWS UNBOUNDED PRECEDING)
//   RETURN {doc, running_total}
//
// CASE EXPRESSIONS:
// FOR doc IN products
//   RETURN {
//     name: doc.name,
//     category: CASE
//       WHEN doc.price < 10 THEN "budget"
//       WHEN doc.price < 100 THEN "mid-range"
//       ELSE "premium"
//     END
//   }
//
// OPTIONAL CHAINING:
// FOR doc IN users
//   RETURN {
//     name: doc.name,
//     city: doc.address?.city,
//     zip: doc.address?.zipcode
//   }
//
// NULLISH COALESCING:
// FOR doc IN users
//   RETURN {
//     display_name: doc.nickname ?? doc.first_name ?? doc.email
//   }
//
// ============================================================================

// ============================================================================
// CONTROLLER EXAMPLES WITH SDBQL
// ============================================================================
//

class ReportsController extends Controller {
    static {
        this.db = solidb_connect("localhost", 6745, "api-key");
        this.database = "myapp";
    }
    
    // Example 1: Aggregation with COLLECT
    fn sales_by_month(req: Any) -> Any {
        let query = `
            FOR sale IN sales
            FILTER sale.date >= @start_date AND sale.date < @end_date
            LET month = DATE_FORMAT(sale.date, "%Y-%m")
            COLLECT month = month 
            AGGREGATE total = SUM(sale.amount), count = COUNT(sale)
            SORT month
            RETURN {month, total, count}
        `;
        
        let params = {
            "start_date": "2024-01-01",
            "end_date": "2024-12-31"
        };
        
        let results = solidb_query(this.db, this.database, query, params);
        
        return {
            "status": 200,
            "body": json_stringify({"monthly_sales": results})
        };
    }
    
    // Example 2: Text Search with Fulltext Index
    fn search_users(req: Any) -> Any {
        let query_params = req["query"];
        let search_term = query_params["q"] ?? "";
        let limit = query_params["limit"] ?? 20;
        
        let query = `
            FOR doc IN users
            SEARCH ANALYZER(doc.bio IN TEXT(@search_term), "text_en")
            LIMIT @limit
            RETURN doc
        `;
        
        let results = solidb_query(this.db, this.database, query, {
            "search_term": search_term,
            "limit": limit
        });
        
        return {
            "status": 200,
            "body": json_stringify({
                "query": search_term,
                "results": results
            })
        };
    }
    
    // Example 3: Geo Query
    fn nearby_stores(req: Any) -> Any {
        let lat = 37.7749;
        let lng = -122.4194;
        let radius_km = 10;
        
        let query = `
            FOR store IN stores
            LET dist = DISTANCE(store.location, [@lng, @lat])
            FILTER dist < @radius * 1000
            SORT dist
            RETURN {
                name: store.name,
                address: store.address,
                distance_km: ROUND(dist / 1000, 2)
            }
        `;
        
        let results = solidb_query(this.db, this.database, query, {
            "lat": lat,
            "lng": lng,
            "radius": radius_km
        });
        
        return {
            "status": 200,
            "body": json_stringify({"stores": results})
        };
    }
    
    // Example 4: Vector Similarity Search
    fn similar_products(req: Any) -> Any {
        let query_params = req["json"];
        let query_embedding = query_params["embedding"];
        let threshold = query_params["threshold"] ?? 0.8;
        let limit = query_params["limit"] ?? 10;
        
        let query = `
            FOR doc IN products
            LET score = VECTOR_COSINE(doc.embedding, @embedding)
            FILTER score >= @threshold
            SORT score DESC
            LIMIT @limit
            RETURN {doc, score}
        `;
        
        let results = solidb_query(this.db, this.database, query, {
            "embedding": query_embedding,
            "threshold": threshold,
            "limit": limit
        });
        
        return {
            "status": 200,
            "body": json_stringify({"products": results})
        };
    }
    
    // Example 5: Graph Traversal (friend of friend)
    fn suggest_friends(req: Any) -> Any {
        let user_id = req["params"]["user_id"];
        
        let query = `
            FOR v, e, p IN 1..2 ANY @user_id GRAPH "friends"
            FILTER v._key != @user_id
            FILTER v.active == true
            LET connection_level = LENGTH(p.edges)
            SORT connection_level, v.name
            LIMIT 10
            RETURN {
                user: v,
                connection_level: connection_level,
                mutual_friends: LENGTH(p.vertices) - 2
            }
        `;
        
        let results = solidb_query(this.db, this.database, query, {
            "user_id": user_id
        });
        
        return {
            "status": 200,
            "body": json_stringify({"suggestions": results})
        };
    }
    
    // Example 6: UPSERT (insert or update)
    fn update_counter(req: Any) -> Any {
        let page_key = req["json"]["page_key"];
        
        let query = `
            UPSERT { _key: @page_key }
            INSERT { _key: @page_key, views: 1, last_view: DATE_NOW() }
            UPDATE { views: OLD.views + 1, last_view: DATE_NOW() }
            IN page_views
            RETURN NEW
        `;
        
        let result = solidb_query(this.db, this.database, query, {
            "page_key": page_key
        });
        
        return {
            "status": 200,
            "body": json_stringify({"page_view": result})
        };
    }
    
    // Example 7: Subquery with aggregation
    fn user_dashboard(req: Any) -> Any {
        let user_id = req["params"]["user_id"];
        
        let query = `
            LET user = FIRST(FOR u IN users FILTER u._key == @user_id RETURN u)
            
            LET posts_count = LENGTH(
                FOR p IN posts FILTER p.user_id == @user_id RETURN p
            )
            
            LET comments_count = LENGTH(
                FOR c IN comments FILTER c.user_id == @user_id RETURN c
            )
            
            LET total_likes = SUM(
                FOR p IN posts FILTER p.user_id == @user_id RETURN p.likes ?? 0
            )
            
            RETURN {
                user: user,
                stats: {
                    posts: posts_count,
                    comments: comments_count,
                    total_likes: total_likes
                }
            }
        `;
        
        let result = solidb_query(this.db, this.database, query, {
            "user_id": user_id
        });
        
        return {
            "status": 200,
            "body": json_stringify({"dashboard": result})
        };
    }
    
    // Example 8: Window function (running total)
    fn running_sales(req: Any) -> Any {
        let query = `
            FOR sale IN sales
            FILTER sale.date >= @start_date
            SORT sale.date
            LET running_total = SUM(sale.amount) 
                OVER (ORDER BY sale.date ROWS UNBOUNDED PRECEDING)
            RETURN {
                date: sale.date,
                amount: sale.amount,
                running_total: running_total
            }
        `;
        
        let results = solidb_query(this.db, this.database, query, {
            "start_date": "2024-01-01"
        });
        
        return {
            "status": 200,
            "body": json_stringify({"sales_with_totals": results})
        };
    }
    
    // Example 9: JSON operations
    fn parse_user_metadata(req: Any) -> Any {
        let user_id = req["params"]["user_id"];
        
        let query = `
            FOR doc IN users
            FILTER doc._key == @user_id
            RETURN {
                name: doc.name,
                email: doc.email,
                metadata_parsed: JSON_PARSE(doc.metadata),
                setting_theme: JSON_VALUE(doc.settings, "$.theme"),
                has_notifications: IS_BOOL(JSON_VALUE(doc.settings, "$.notifications"))
            }
        `;
        
        let result = solidb_query(this.db, this.database, query, {
            "user_id": user_id
        });
        
        return {
            "status": 200,
            "body": json_stringify({"user": result})
        };
    }
    
    // Example 10: Case expressions and type checks
    fn categorize_products(req: Any) -> Any {
        let query = `
            FOR doc IN products
            RETURN {
                name: doc.name,
                price: doc.price,
                category: CASE
                    WHEN doc.price < 10 THEN "budget"
                    WHEN doc.price < 100 THEN "mid-range"
                    WHEN doc.price < 1000 THEN "premium"
                    ELSE "luxury"
                END,
                in_stock: IS_NUMBER(doc.stock) AND doc.stock > 0,
                discount_applied: doc.original_price != null 
                    ? ROUND((1 - doc.price / doc.original_price) * 100, 0)
                    : null
            }
        `;
        
        let results = solidb_query(this.db, this.database, query, {});
        
        return {
            "status": 200,
            "body": json_stringify({"products": results})
        };
    }
}

// ============================================================================
// LUA SCRIPTING EXAMPLES
// ============================================================================
//
// -- Lua script for custom endpoint
// function handle(request)
//     local db = DB.connect()
//     
//     -- Build complex query
//     local users = db:query(
//         "FOR u IN users FILTER u.active == @active RETURN u",
//         {active = true}
//     )
//     
--     -- Insert audit log
//     db.audit_logs:insert({
//         action = "user_list",
//         count = #users,
//         timestamp = os.time()
//     })
//     
//     return {
//         status = 200,
//         body = json.encode({users = users})
//     }
// end
//
// -- Lua transaction example
// function transfer_funds(request)
//     local db = DB.connect()
//     local amount = request.body.amount
//     local from_id = request.body.from
//     local to_id = request.body.to
//     
//     local result = db:transaction(function(tx)
//         -- Check balance
//         local from = tx.accounts:get(from_id)
//         if not from or from.balance < amount then
//             return {error = "Insufficient funds"}
//         end
//         
//         -- Transfer
//         tx.accounts:update(from_id, {balance = from.balance - amount})
//         tx.accounts:update(to_id, {balance = tx.accounts:get(to_id).balance + amount})
//         
//         -- Log
//         tx.transfers:insert({
//             from = from_id,
//             to = to_id,
//             amount = amount,
//             timestamp = os.time()
//         })
//         
//         return {success = true}
//     end)
//     
//     return {
//         status = result.success and 200 or 400,
//         body = json.encode(result)
//     }
// end
//
// ============================================================================

