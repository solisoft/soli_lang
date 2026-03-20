# Soli: A Minimal Language for Web Development

Web development doesn't need to be complex. Soli is designed from the ground up to be a minimal, focused language that handles web development natively.

## Philosophy: Less is More

Most languages for web development started as general-purpose languages that accumulated web frameworks over decades. Soli takes a different approach:

- **No hidden complexity** - Every feature exists because web developers need it
- **Single executable** - No need for npm, pip, gem, or bundlers
- **Batteries included** - ORM, routing, sessions, and templating built-in
- **Fast by default** - Performance isn't an afterthought

## A Language That Fits in Your Head

Soli has fewer keywords than most languages:

```soli
# A complete web route
get("/users", "users#index")

# A model with validation
class User extends Model
    name: String
    email: String
    
    validates("email", {"presence": true, "format": "email"})
end
```

No boilerplate. No configuration files. No invisible magic.

## Soli's Core: Rust + AI

Soli itself is written in Rust - a language known for performance and safety. But Rust's complexity can be a barrier. That's where AI comes in:

### Better Rust with AI

Writing raw Rust involves:
- Complex lifetimes and borrowing
- Verbose type annotations  
- Manual memory management concepts

With AI assistance, we can focus on **what** the code should do, not **how** it manages memory:

```rust
// AI can generate this from a simple spec:
// "create a web server that handles 10k concurrent connections"

async fn handle_request(req: Request) -> Result<Response, Error> {
    // AI handles the async complexity
    let handler = RouteHandler::new();
    handler.route(req).await
}
```

### Why Soli's Core Benefits from AI

1. **Boilerplate Reduction** - AI generates the repetitive Rust patterns
2. **Safety Checks** - AI catches borrow checker errors before they happen
3. **Performance Tuning** - AI suggests optimal data structures
4. **Documentation** - AI auto-generates docs from implementation

### The Hybrid Approach

Soli demonstrates a new development model:
- **Core language** - Written in Rust for performance (with AI help)
- **Application logic** - Written in Soli for simplicity
- **AI assists** - Both levels benefit from intelligent code generation

This means even as Soli grows, it stays approachable - the complexity lives in the core (where AI helps), not in the user-facing language.

## Getting Started

```bash
# Install Soli
curl -sSL https://raw.githubusercontent.com/solisoft/soli_lang/main/install.sh | sh

# Create a new project
soli new myapp

# Run it
cd myapp && soli serve
```

That's it. No backend dependencies - TailwindCSS comes pre-configured in the template for styling, but works out of the box.

## Conclusion

Soli isn't trying to be the most powerful language. It's trying to be the most **useful** language for web development - a language that AI agents can easily understand, improve, and extend.

The web doesn't need more complexity. It needs a language that does the hard work so you don't have to.