# No Build, No Dependency: Why It Matters for Security and Simplicity

<figure style="margin:1.5rem auto;max-width:1200px;">
  <picture>
    <source type="image/webp" srcset="/images/blog/no-build-no-dependency.webp">
    <img src="/images/blog/no-build-no-dependency.png" width="1536" height="1024" alt="A single clean Rust binary glowing at the center of a sparse diagram, surrounded by crossed-out npm dependency graphs and build tool logos." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  </picture>
</figure>

Every modern web stack has a dependency graph. Most developers treat this as a fact of life, like taxes or log rotation. You install a framework, and npm quietly installs 800 packages to power it. You install a build tool, and that build tool has its own transitive tree. You upgrade one package, and three others break. The lock file grows, the `node_modules` folder swells past a gigabyte, and at some point you stop counting.

Soli takes the opposite bet: the entire stack ships as a single binary, and your application has no package manager, no `node_modules`, no bundler, and no build step.

This is not a limitation. It is a deliberate design decision — and it has concrete security and operational consequences.

## The npm Supply-Chain Threat Landscape

The Node and npm ecosystem has made software supply-chain attacks a mainstream concern. The threat is not hypothetical.

In 2022, the `colors` and `faker` libraries — with hundreds of millions of weekly downloads combined — were intentionally sabotaged by their own author, breaking projects worldwide overnight. The `event-stream` incident a few years earlier saw a malicious contributor publish a version that silently harvested cryptocurrency wallets. In 2024, researchers found dozens of typosquatting packages mimicking popular names like `expresss`, `lodahs`, and `reqeust`, all shipping credential exfiltration code. Protestware packages have injected anti-war messages, corrupted files on specific geolocations, and in one case attempted to delete the filesystem of machines in certain regions.

These are not edge cases. They are the natural consequence of a model where a typical production application depends on thousands of packages from hundreds of maintainers, most of them maintained voluntarily, with no security review gate.

Your application's attack surface is not just your code. It is everything in your `node_modules`.

## The Build-Tool Complexity Tax

Beyond security, the dependency model imposes an operational tax that compounds over time.

A typical TypeScript web application requires:

- A package manager (npm, pnpm, or yarn) with its own lockfile format and edge cases
- A transpiler (tsc, esbuild, swc, Babel) that turns TypeScript into JavaScript
- A bundler (webpack, Vite, Rollup, Parcel) that turns modules into deployable assets
- A CSS preprocessor or utility framework with its own build pipeline
- A dev server that watches for changes and re-runs the pipeline
- Configuration files for each of these tools, often dozens of lines each
- CI steps to install, build, lint, and test across a clean environment
- Runtime environment alignment (Node versions, package versions across machines)

Most of this machinery exists not because the application needs it, but because the language and ecosystem were not designed for web development end to end. You are assembling a toolchain from independent parts that have to be glued together.

Every new team member needs to install all of it. Every deployment needs to run it. Every CI build spends time in `npm install`. Every lockfile update is a risk surface.

## What Soli Replaces This With

Soli ships a single Rust binary. That binary includes:

- The language interpreter and runtime
- The HTTP server (170k+ req/sec on a single core)
- The MVC router and controller dispatch
- The ORM and database connection pool
- The ERB-style template engine
- The session manager
- The background job runner
- The test runner
- The linter

There is no install step for any of these. When you run `soli serve`, you are running one process. When you deploy, you copy one file.

```soli
# A complete Soli route file — no imports required for the framework itself
get("/", "home#index")
post("/users", "users#create")
resources("posts")
```

```soli
# A model with validation — the ORM is part of the runtime
class User < Model
  name: String
  email: String

  validates("email", {"presence": true, "format": "email"})
end
```

There is no Gemfile, no requirements.txt, no package.json. The framework is not a library you install into a language runtime. The framework and the language are the same thing.

## The Security Benefits Are Structural

When you eliminate the dependency graph, you eliminate entire categories of supply-chain risk:

**No transitive dependencies to compromise.** If an attacker cannot inject a malicious package into your build, they cannot reach your production process through that vector. A Soli application does not run arbitrary npm packages at install time, at build time, or at runtime.

**Reproducible builds without effort.** The same `soli` binary produces the same behavior regardless of network state, registry availability, or cache freshness. There is no equivalent of `npm install --legacy-peer-deps` or "works on my machine because my node_modules was from last week."

**Smaller audit surface.** Security audits of Soli applications focus on application logic and configuration. There is no 800-package dependency tree to review. The runtime itself is open source and written in Rust, where the compiler enforces memory safety at the language level.

**No protestware risk.** Package authors cannot ship an update that corrupts your application because Soli's runtime is not sourced from a public registry at deploy time. You pin the `soli` binary version the same way you pin a Docker base image.

## Simplicity Is Not a Compromise

The argument against zero-dependency stacks is usually that you lose flexibility. If everything is built in, you cannot swap the router, cannot pick a different ORM, cannot upgrade the template engine independently.

That is true. And for most web applications, it is a good trade.

The flexibility of the npm model is real, but it is mostly used to solve problems the npm model itself creates. You switch from webpack to Vite because webpack is slow. You switch from one validation library to another because the API changed in a major version. You add an abstraction layer over your HTTP client because the underlying library's maintainer went inactive. Most of this churn has nothing to do with what your application actually does.

Soli's constraints are the framework's design. When there is one way to define a route, one way to define a model, one way to write a template, team members can read each other's code without a dictionary. New contributors can be productive quickly. AI coding agents can navigate the codebase without needing to understand which version of which library introduced which behavior.

## What This Means in Production

Zero dependencies means zero dependency updates to manage in production. There is no scheduled `npm audit fix` run, no Dependabot pull requests, no CVE triage for packages you use indirectly three levels deep.

When a security issue is found in Soli itself, it is fixed in the Soli binary and you upgrade one artifact. When a security issue is found in a transitive npm dependency, you upgrade the npm package that depends on the npm package that depends on the vulnerable package, verify that no peer dependency constraints break, run the build pipeline, and hope the lock file resolves the same way in CI as it did locally.

The operational model is simpler because the mental model is simpler.

## The Deliberate Choice

Soli is not no-build because it has not gotten around to adding a build step. It is no-build because that is the design.

The choice shapes everything: the language is compact because it does not need to accommodate every use case a plugin might cover. The runtime is fast because it is not shelling out to Node to run JavaScript. The deployment story is simple because there is one artifact to move.

That is not for everyone. If your team needs a specific third-party library that does not have a Soli built-in equivalent, Soli is not the right tool yet. If your organization's tooling is deeply integrated with the npm ecosystem, switching has a real cost.

But if you are starting a new server-rendered web application, the question is worth asking: how much of the complexity in your current stack exists to serve the application, and how much exists to serve the toolchain?

Soli bets that for most teams, the answer is more than you think.
