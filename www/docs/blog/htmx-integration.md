# HTMx: The Missing Link Between Traditional MVC and Modern Interactivity

If you've been building web apps for a while, you probably remember when everything was simple: a form posts to an endpoint, the server processes it, returns HTML, the page refreshes. Then SPA frameworks arrived and everything got complicated.

HTMx brings us back to simplicity while still allowing dynamic, interactive applications.

## What is HTMx?

HTMx is a JavaScript library that extends HTML with modern capabilities. Instead of learning a new syntax, you just use HTML attributes:

```html
<button hx-get="/api/users" hx-target="#user-list">
    Load Users
</button>
```

That's it. No JavaScript, no frameworks, no build steps.

## Why HTMx for Soli?

Soli already has LiveView for real-time interactivity, but not everyone needs WebSocket connections. HTMx is perfect when you want:

- Simple HTTP request/response patterns
- No WebSocket overhead
- Progressive enhancement of plain HTML
- Small bundle size (~14KB vs React's 100KB+)

## Getting Started

First, add HTMx to your layout:

```soli
# www/app/views/layouts/application.html.slv
<script src="<%= public_path("js/htmx.min.js") %>"></script>
```

Download HTMx from [htmx.org](https://htmx.org) and place it in `public/js/`.

## HTMx Helper Functions

To make HTMx even easier to use in Soli, let's create some helper functions:

```soli
# stdlib/htmx.sl

fn hx_get(url)
    'hx-get="' + url + '"'
end

fn hx_post(url)
    'hx-post="' + url + '"'
end

fn hx_target(selector)
    'hx-target="' + selector + '"'
end

fn hx_swap(method)
    'hx-swap="' + method + '"'
end

fn hx_trigger(event)
    'hx-trigger="' + event + '"'
end

fn hx_push_url(enabled)
    'hx-push-url="' + (enabled ? "true" : "false") + '"'
end
```

Now using HTMx in your templates is clean and readable:

```soli
<button <%= hx_get("/users") %>>Load Users</button>

<div id="user-list"></div>
```

## Example: Todo List

Let's build a simple todo list with HTMx:

```soli
# app/controllers/todos_controller.sl

fn index(req)
    let todos = Todo.all
    render("todos/index", {"todos": todos})
end

fn create(req)
    let params = req["all"]
    let todo = Todo.create({"title": params["title"], "done": false})
    
    render("todos/_todo", {"todo": todo})
end

fn toggle(req)
    let id = req["all"]["id"]
    let todo = Todo.find(id)
    todo["done"] = !todo["done"]
    todo.save
    
    render("todos/_todo", {"todo": todo})
end
```

```soli
# app/views/todos/index.html.slv

<h1>My Todos</h1>

<form <%= hx_post("/todos", target: "#todos") %>>
    <input type="text" name="title" placeholder="New todo...">
    <button type="submit">Add</button>
</form>

<div id="todos">
    <%= render("_todos", {"todos": todos}) %>
</div>
```

```soli
# app/views/todos/_todo.html.slv

<div class="todo <%= todo["done"] ? "completed" : "" %>">
    <input 
        type="checkbox" 
        <%= todo["done"] ? "checked" : "" %>
        <%= hx_patch("/todos/" + todo["id"], target: "#todo-" + todo["id"]) %>
    >
    <span><%= todo["title"] %></span>
</div>
```

The server returns only the partial HTML fragment. HTMx swaps it into the page - no full page refresh, no client-side rendering.

## Why This Matters

1. **Server-side rendering** - Your HTML is generated on the server, where you have all your database connections, business logic, and security

2. **No JS knowledge required** - You write templates, not JavaScript components

3. **Progressive enhancement** - Works even if JS is disabled (mostly)

4. **Small footprint** - 14KB total, no dependencies

5. **SEO friendly** - Full HTML is served on initial load

## When to Choose HTMx vs LiveView

| Use HTMx when... | Use LiveView when... |
|------------------|---------------------|
| Simple request/response | Real-time updates needed |
| Standard CRUD operations | Frequent state changes |
| Forms and page navigations | Collaborative features |
| You want simplicity | You need WebSocket performance |

## Conclusion

HTMx brings the simplicity of traditional server-side rendering to modern web development. Combined with Soli's clean syntax, you get:

- Expressive templates with HTMx helpers
- Server-rendered HTML with partials
- Progressive enhancement by default
- No JavaScript framework complexity

It's not about replacing JavaScript - it's about using the right tool for the right job. Sometimes that's React. Sometimes it's a 14KB library that works with HTML you already know.