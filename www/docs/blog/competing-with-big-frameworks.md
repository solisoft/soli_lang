# How Soli Competes with Big Frameworks

<figure style="margin:1.5rem auto;max-width:1200px;">
  <picture>
    <source type="image/webp" srcset="/images/blog/competing-with-big-frameworks.webp">
    <img src="/images/blog/competing-with-big-frameworks.png" width="1536" height="1024" alt="A glowing hexagonal Soli core radiating brand-color light, surrounded by tall stacked tower-blocks representing the layered complexity of bigger frameworks." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  </picture>
</figure>

Every web framework competes with the same giants: Rails, Laravel, Django, Spring, Next.js, NestJS, and the rest of the ecosystems that have already solved a lot of hard problems.

Soli does not win by pretending those frameworks are bad. They are mature, proven, and full of good ideas. Soli competes by making a different bet: for many teams, the fastest way to build a web product is not more layers, more packages, and more generated glue. It is a small language and framework designed together around the shape of server-side web apps.

## The Big Framework Advantage

Large frameworks are popular for good reasons:

- They have huge ecosystems
- They have years of production history
- They have answers for almost every edge case
- They have large hiring pools
- They have plugins, templates, and tutorials for everything

That maturity matters. If your company needs ten third-party integrations on day one, or if your team already has deep expertise in one ecosystem, a big framework can be the practical choice.

But maturity also has a cost.

## Where Big Frameworks Get Heavy

As frameworks grow, they often accumulate complexity around the edges:

- A language runtime that was not designed specifically for web apps
- A package manager and dependency graph before the first route exists
- Separate tools for routing, templating, background jobs, validation, sessions, database access, assets, testing, and deployment
- Client-side build systems even for mostly server-rendered apps
- Multiple layers of conventions that new developers need to learn before they can change a page

None of this is automatically wrong. Complexity can be worth it. But a lot of web applications do not need a cathedral of moving parts to render HTML, read params, validate input, write to a database, and return a response.

That is the space where Soli competes.

## Soli's Bet: One Coherent System

Soli is both a language and a framework. That changes the design space.

Instead of adapting a general-purpose language to the web, Soli can make web development feel native:

```soli
get("/users", "users#index")

class User < Model
  name: String
  email: String

  validates("email", {"presence": true, "format": "email"})
end
```

Routes, controllers, views, models, validation, sessions, jobs, uploads, and testing live in one coherent environment. The goal is not to hide everything. The goal is to remove the glue code that appears only because unrelated tools had to be stitched together.

## Competing on Speed of Understanding

Big frameworks often compete on raw capability. Soli competes on how quickly a developer can understand the whole application.

That matters because most product work is not typing code. It is answering questions:

- Where does this route go?
- Which controller handles this form?
- What params are accepted?
- Where is the validation?
- What gets rendered?
- How do I test this behavior?

Soli keeps those answers close to the code. A smaller language surface makes it easier for humans to read, and easier for AI coding agents to modify safely.

## Performance Still Matters

Soli is written in Rust, so performance is part of the foundation rather than an afterthought. That does not mean every Soli app automatically beats every app written in Rails, Laravel, Django, or Node. Real performance depends on database queries, caching, templates, network latency, and deployment choices.

But starting from a fast runtime gives Soli room to stay simple without becoming slow. You should not need a microservice split, a complicated cache hierarchy, or a frontend rewrite just to make a normal server-rendered app feel responsive.

## Batteries Included, Without the Sprawl

Soli's competitive edge is not that it has more features than the big frameworks. It is that the common features are designed to feel like part of the same tool:

- MVC routing and controllers
- ERB-style templates
- Models and database helpers
- Sessions and authentication building blocks
- Validation
- File uploads and image transforms
- Background jobs
- Testing utilities
- Built-in helpers for common web tasks

In a large framework, these may come from different libraries with different conventions. In Soli, the intent is that they compose naturally.

## The Honest Tradeoff

Soli is younger than the big frameworks. That means fewer tutorials, fewer Stack Overflow answers, fewer third-party packages, and fewer production stories.

The tradeoff is focus.

Soli can move quickly because it does not need to preserve decades of ecosystem behavior. It can choose the simpler API. It can make the common path first-class. It can optimize for AI-assisted development from the beginning instead of adapting to it later.

## When Soli Is the Right Choice

Soli is a strong fit when you want:

- A server-rendered web app without a large JavaScript stack
- A compact language that is easy to read and change
- Rails-style productivity with a smaller surface area
- Rust-backed performance without writing Rust application code
- A framework that AI agents can navigate without fighting layers of indirection
- One toolchain for routes, models, templates, jobs, tests, and common web features

It is probably not the right choice when the ecosystem matters more than simplicity, when your team already depends heavily on an existing framework's plugin world, or when you need a very specific integration that Soli does not support yet.

## The Real Competition

Soli is not trying to be a bigger Rails, a smaller Django, or a server-side Next.js clone.

It competes against the feeling that web development has to be complicated.

Big frameworks are built for a wide world of use cases. Soli is built for the most common one: turning ideas into maintainable web applications with as little ceremony as possible.

That is how Soli competes. Not by being everything, but by making the important things feel close, fast, and understandable.
