# Semantic Search with `.similar()` in Soli

Building a search that understands meaning — not just keywords — used to require a dedicated search service, a separate indexing pipeline, and a lot of infrastructure. Soli's `.similar()` method changes that by making vector similarity search a first-class database primitive.

With a single method chain you can rank any database query by semantic relevance:

```soli
results = Product
    .where("category == 'electronics'")
    .similar("wireless noise-cancelling headphones under 200")
    .all

for product in results
    print(product.name + " — " + str(product._similarity_score))
end
```

Each result gets a `_similarity_score` field (0.0 to 1.0) so you can surface relevance to your users.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/similar-search.jpg" width="1024" height="576" alt="Semantic vector search in Soli: a natural language query is embedded and compared via cosine similarity against document vectors stored in the database, returning the most relevant results ranked by meaning." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">`.similar()` turns natural language into ranked results using built-in vector similarity — no extra services required.</figcaption>
</figure>

## How It Works

The `.similar()` method is a QueryBuilder chain method. When you call it:

1. **Embedding generation** — The query text is sent to an OpenAI-compatible API to produce a vector embedding
2. **Document fetch** — Matching documents are fetched from SolidB (all existing filters, joins, and conditions apply)
3. **Cosine similarity** — Each document's embedding field is compared against the query embedding using cosine similarity
4. **Ranking** — Results are sorted by similarity (highest first), trimmed to top-K, and returned with a `_similarity_score` on each record

SolidDB underpins this with native vector search capabilities — HNSW indexes, `VECTOR_SIMILARITY()` in SDBQL, scalar quantization, and a dedicated REST API. The current Soli runtime computes similarity client-side, but the same SolidDB engine that stores your data is ready for native vector workloads at scale.

## Configuration

Set these environment variables:

| Variable | Default | Required |
|----------|---------|----------|
| `SOLI_EMBEDDING_API_KEY` | — | Yes |
| `SOLI_EMBEDDING_URL` | `https://api.openai.com/v1/embeddings` | No |
| `SOLI_EMBEDDING_MODEL` | `text-embedding-3-small` | No |

When the API key is not set, `.similar()` returns an empty result set.

## API

```soli
.similar(query_text, field?, top_k?)
```

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `query_text` | `String` | — | The natural-language search query |
| `field` | `String` | `"embedding"` | The document field containing the embedding vector |
| `top_k` | `Int` | `10` | Maximum number of results to return |

## Basic Usage

### Simple semantic search

Find the 10 most semantically similar posts, using the default `embedding` field:

```soli
results = Post
    .where("published == true")
    .similar("how to deploy a web app")
    .all

for post in results
    print(post.title + " (score: " + str(post._similarity_score) + ")")
end
```

### Custom embedding field and top-K

If your model stores embeddings in a different field, or you need more (or fewer) results:

```soli
results = Product
    .where("active == true")
    .similar("red running shoes", "title_embedding", 5)
    .all

results.each(fn(p)
    print(p.name + " — " + str(p._similarity_score))
end)
```

### Combining with other chain methods

`.similar()` composes with every other QueryBuilder method — `.where()`, `.order()`, `.includes()`, `.limit()`, and so on:

```soli
results = Product
    .includes("reviews")
    .where("price <= @max", {"max": 100})
    .where("category == 'footwear'")
    .similar("comfortable hiking boots", "description_embedding", 20)
    .order("price", "asc")
    .all
```

## Tutorial: Product Recommendation Engine

Let's build a complete product search endpoint that combines traditional filters with semantic relevance. We'll seed products, auto-generate embeddings, and serve ranked results.

### Step 1: Define the model

```soli
# app/models/product.sl
class Product extends Model {
    # Embedding field stores the vector for similarity search
    # It's populated automatically when the product is created
}
```

### Step 2: Create the migration

Generate and write the migration to create the `products` collection:

```bash
soli db:migrate generate create_products
```

```soli
# db/migrations/20260101000000_create_products.sl

fn up(db: Any)
    db.create_collection("products")

    # Create a vector index on the embedding field for HNSW similarity search
    db.create_vector_index("products", "embedding_idx", "embedding", 1536)
end

fn down(db: Any)
    db.drop_vector_index("products", "embedding_idx")
    db.drop_collection("products")
end
```

Apply it:

```bash
soli db:migrate
```

SolidDB uses **HNSW (Hierarchical Navigable Small World)** graphs for fast approximate nearest-neighbor search. With a vector index, search becomes O(log n) instead of O(n) with ~95%+ recall. You can also enable **scalar quantization** by adding `"quantization": "scalar"` to reduce memory 4x.

> **Note:** The current `.similar()` implementation computes similarity client-side in Rust. The vector index is available for when you use SolidDB's native `VECTOR_SIMILARITY()` in raw AQL queries or the dedicated vector search REST API.

### Step 3: Seed the data with embeddings

Populate products and generate embeddings via the OpenAI-compatible API:

```soli
# db/seeds.sl

fn generate_embedding(text)
    let api_key = getenv("SOLI_EMBEDDING_API_KEY")
    let url = getenv("SOLI_EMBEDDING_URL") rescue "https://api.openai.com/v1/embeddings"
    let model = getenv("SOLI_EMBEDDING_MODEL") rescue "text-embedding-3-small"

    let response = http_post(url, {
        "headers": {
            "Authorization": "Bearer " + api_key,
            "Content-Type": "application/json"
        },
        "body": json_stringify({
            "input": text,
            "model": model
        })
    })

    let body = json_parse(response["body"])
    return body["data"][0]["embedding"]
end

# Seed products
let products = [
    {"name": "Ultra Comfort Running Shoes", "description": "Lightweight mesh running shoes with cushioned sole for long-distance runners", "category": "footwear", "price": 129.99},
    {"name": "Trail Blazer Hiking Boots", "description": "Waterproof leather hiking boots with reinforced toe and ankle support", "category": "footwear", "price": 189.99},
    {"name": "Wireless Noise-Cancelling Headphones", "description": "Over-ear headphones with active noise cancellation and 30-hour battery", "category": "electronics", "price": 249.99},
    {"name": "Smart Fitness Watch", "description": "Water-resistant fitness tracker with heart rate monitor and GPS", "category": "electronics", "price": 199.99},
    {"name": "Organic Cotton T-Shirt", "description": "Soft organic cotton crew-neck t-shirt available in 12 colors", "category": "clothing", "price": 34.99},
    {"name": "Merino Wool Sweater", "description": "Lightweight merino wool sweater perfect for layering in cold weather", "category": "clothing", "price": 89.99},
    {"name": "Portable Bluetooth Speaker", "description": "Rugged waterproof speaker with 20-hour battery and deep bass", "category": "electronics", "price": 79.99},
    {"name": "Yoga Mat Premium", "description": "Extra-thick non-slip yoga mat with carrying strap", "category": "fitness", "price": 49.99}
]

for p in products
    let combined_text = p["name"] + ". " + p["description"] + ". Category: " + p["category"]
    let embedding = generate_embedding(combined_text)

    Product.create({
        "name": p["name"],
        "description": p["description"],
        "category": p["category"],
        "price": p["price"],
        "embedding": embedding
    })

    print("Created: " + p["name"])
end
```

### Step 4: Build the search controller

```soli
# app/controllers/products_controller.sl

fn search(req)
    let query = req["params"]["q"] || req["json"]["query"]
    let category = req["params"]["category"]
    let max_price = req["params"]["max_price"]
    let top_k = int(req["params"]["top_k"] rescue "10")

    if !query || query == ""
        return json_response({"error": "query parameter is required"}, 422)
    end

    # Build the query chain dynamically
    let q = Product.where("true == true")

    if category && category != ""
        q = q.where("category == @cat", {"cat": category})
    end

    if max_price && max_price != ""
        q = q.where("price <= @max", {"max": float(max_price)})
    end

    # Add semantic search — uses the default "embedding" field
    q = q.similar(query, "embedding", top_k)

    let results = q.all

    let output = results.map(fn(p)
        return {
            "name": p.name,
            "description": p.description,
            "category": p.category,
            "price": p.price,
            "score": p._similarity_score
        }
    end)

    return json_response({
        "query": query,
        "count": len(output),
        "results": output
    })
end
```

### Step 5: Wire up the route

```soli
# config/routes.sl
get("/products/search", "products#search")
```

### Step 6: Try it out

```bash
curl "http://localhost:5011/products/search?q=comfortable+footwear+for+running&max_price=150"
```

Response:

```json
{
  "query": "comfortable footwear for running",
  "count": 3,
  "results": [
    {
      "name": "Ultra Comfort Running Shoes",
      "description": "Lightweight mesh running shoes with cushioned sole for long-distance runners",
      "category": "footwear",
      "price": 129.99,
      "score": 0.924
    },
    {
      "name": "Trail Blazer Hiking Boots",
      "description": "Waterproof leather hiking boots with reinforced toe and ankle support",
      "category": "footwear",
      "price": 189.99,
      "score": 0.781
    },
    {
      "name": "Merino Wool Sweater",
      "description": "Lightweight merino wool sweater perfect for layering in cold weather",
      "category": "clothing",
      "price": 89.99,
      "score": 0.412
    }
  ]
}
```

Results are ranked by semantic relevance. The running shoes match best, the hiking boots also rank highly (both are footwear), and the sweater scores lower because it's a different category — but still semantically related to "comfortable."

### Step 7: Surface scores in the UI

In your ERB template, show the relevance score:

```erb
<% for product in results %>
    <div class="product-card">
        <h3><%= h(product["name"]) %></h3>
        <p><%= h(product["description"]) %></p>
        <span class="price">$<%= product["price"] %></span>
        <% if product["score"] %>
            <span class="badge">Relevance: <%= round(product["score"] * 100) %>%</span>
        <% end %>
    </div>
<% end %>
```

## Combining with Eager Loading

`.similar()` works alongside `.includes()` so you can avoid N+1 queries when displaying related data:

```soli
results = Product
    .includes("reviews", "category")
    .similar("durable outdoor gear", "description_embedding", 20)
    .all

for product in results
    print(product.name + " (" + product.reviews.length + " reviews)")
end
```

## Native Vector Search with SolidDB

Beyond `.similar()`, SolidDB provides a full vector search engine that you can use directly for production-scale workloads:

### SDBQL Vector Functions

Use `VECTOR_SIMILARITY()` in raw AQL queries to push similarity computation to the database:

```sql
FOR doc IN products
  LET sim = VECTOR_SIMILARITY(doc.embedding, @query_vec)
  FILTER sim > 0.7
  FILTER doc.category == "electronics"
  SORT sim DESC
  LIMIT 20
  RETURN MERGE(doc, { "_similarity_score": sim })
```

### REST Vector Search API

Query a vector index directly without writing SDBQL:

```bash
curl -X POST \
  http://localhost:6745/_api/database/solidb/vector/products/embedding_idx/search \
  -H "Content-Type: application/json" \
  -d '{
    "vector": [0.0123, -0.0456, ...],
    "limit": 10,
    "ef_search": 100
  }'
```

### Hybrid Search

Combine vector similarity with fulltext search for 15-30% better relevance in RAG applications:

```sql
LET results = HYBRID_SEARCH(
    "products",
    "embedding_idx",
    "description",
    @query_vector,
    "comfortable hiking boots",
    { vector_weight: 0.6, text_weight: 0.4, limit: 20 }
)
FOR r IN results
  RETURN { name: r.doc.name, score: r.score, sources: r.sources }
```

See the [SolidDB Vector Search docs](https://solidb.solisoft.net/docs/vector-search) and [Hybrid Search docs](https://solidb.solisoft.net/docs/hybrid-search) for full details.

## Performance Considerations

- The embedding API call adds latency proportional to the query text length (typically 200–500ms for OpenAI's `text-embedding-3-small`)
- The current `.similar()` implementation computes cosine similarity client-side in Rust across all matching documents — this is fast (microseconds per document) but for very large result sets (100k+ documents), consider adding filters to narrow candidates first or switching to SolidDB's native `VECTOR_SIMILARITY()` via raw AQL
- With a vector index and native SolidDB vector search, search becomes O(log n) instead of O(n) using HNSW graphs (~95%+ recall)
- The `ef_search` parameter (default 40) tunes the speed-recall tradeoff at query time

## Summary

| Pattern | Description |
|---------|-------------|
| `.where(...).similar("text").all` | Filtered semantic search (default `embedding` field, top 10) |
| `.where(...).similar("text", "field", N).all` | Custom embedding field and result count |
| `._similarity_score` | Float (0.0–1.0) injected into each result record |

Vector search doesn't have to mean adding Elasticsearch, Pinecone, or a separate AI pipeline. With Soli's `.similar()` method, it's just another link in the query chain — no different from `.where()` or `.order()`. Configure your embedding API key and you're ready to ship semantic search.
