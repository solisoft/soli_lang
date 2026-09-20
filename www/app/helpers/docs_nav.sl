# The order of the documentation, once.
#
# Both the sidebar (`layouts/_docs_nav.html.slv`) and the previous/next arrows
# at the foot of every page (`layouts/_docs_page_nav.html.slv`) render from
# this list, so the two cannot disagree about what comes after what.
#
# They used to. The footers were written by hand, page by page, and **34 of the
# 46 builtins pages disagreed with the sidebar** — nine pointing somewhere else
# entirely, and PASETO and VAPID both claiming to sit between JWT and Regex, so
# a reader paging through met whichever they had opened. A hundred other pages
# had no arrows at all.
#
# Adding a page means adding one line here. Nothing else knows the order.

def docs_nav_sections()
  [
    {
      "title": "Getting Started",
      "icon": "<svg class=\"w-4 h-4 text-lime-400\" fill=\"none\" viewBox=\"0 0 24 24\" stroke=\"currentColor\"><path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"M13 10V3L4 14h7v7l9-11h-7z\" /></svg>",
      "items": [
        { "path": "/docs/getting-started/introduction", "label": "Introduction" },
        { "path": "/ai", "label": "AI" },
        { "path": "/docs/getting-started/installation", "label": "Installation" },
        { "path": "/docs/getting-started/configuration", "label": "Configuration" },
        { "path": "/docs/getting-started/comparison", "label": "How Soli Compares" },
        { "path": "/docs/getting-started/benchmarks", "label": "Benchmarks" },
        { "path": "/docs/getting-started/changelog", "label": "Changelog" }
      ]
    },
    {
      "title": "Core Concepts",
      "icon": "<svg class=\"w-4 h-4 text-blue-400\" fill=\"none\" viewBox=\"0 0 24 24\" stroke=\"currentColor\"><path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"M20 7l-8-4-8 4m16 0l-8 4m8-4v10l-8 4m0-10L4 7m8 4v10M4 7v10l8 4\" /></svg>",
      "items": [
        { "path": "/docs/core-concepts/routing", "label": "Routing" },
        { "path": "/docs/core-concepts/controllers", "label": "Controllers" },
        { "path": "/docs/core-concepts/middleware", "label": "Middleware" },
        { "path": "/docs/core-concepts/views", "label": "Views" },
        { "path": "/docs/core-concepts/forms", "label": "Forms & CSRF" },
        { "path": "/docs/core-concepts/websockets", "label": "WebSockets" },
        { "path": "/docs/core-concepts/streaming", "label": "Streaming & SSE" },
        { "path": "/docs/core-concepts/liveview", "label": "Live View" },
        { "path": "/docs/core-concepts/client-interactivity", "label": "Client Interactivity" },
        { "path": "/docs/core-concepts/i18n", "label": "Internationalization" },
        { "path": "/docs/core-concepts/request-params", "label": "Request Parameters" },
        { "path": "/docs/core-concepts/error-pages", "label": "Error Pages" },
        { "path": "/docs/core-concepts/engines", "label": "Engines" },
        { "path": "/docs/core-concepts/feature-flags", "label": "Feature Flags" }
      ]
    },
    {
      "title": "EUI",
      "icon": "<svg class=\"w-4 h-4 text-blue-400\" fill=\"none\" viewBox=\"0 0 24 24\" stroke=\"currentColor\"><path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z\" /></svg>",
      "items": [
        { "path": "/docs/eui/overview", "label": "Overview" },
        { "path": "/docs/eui/styling", "label": "Styling & roles" },
        { "path": "/docs/eui/events", "label": "Events" },
        { "path": "/docs/eui/assets", "label": "Assets & lists" },
        { "path": "/docs/eui/widgets-layout", "label": "Widgets: Layout" },
        { "path": "/docs/eui/widgets-input", "label": "Widgets: Input" },
        { "path": "/docs/eui/widgets-data", "label": "Widgets: Content" },
        { "path": "/docs/eui/widgets-internals", "label": "Widgets: Internals" }
      ]
    },
    {
      "title": "Database",
      "icon": "<svg class=\"w-4 h-4 text-emerald-400\" fill=\"none\" viewBox=\"0 0 24 24\" stroke=\"currentColor\"><path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4m0 5c0 2.21-3.582 4-8 4s-8-1.79-8-4\" /></svg>",
      "items": [
        { "path": "/docs/database/configuration", "label": "Configuration" },
        { "path": "/docs/database/multi-database", "label": "Multiple Databases" },
        { "path": "/docs/database/postgres", "label": "PostgreSQL" },
        { "path": "/docs/database/mysql", "label": "MySQL" },
        { "path": "/docs/database/sqlite", "label": "SQLite" },
        { "path": "/docs/database/models", "label": "Models & ORM" },
        { "path": "/docs/database/query-builder", "label": "Query Builder" },
        { "path": "/docs/database/relationships", "label": "Relationships" },
        { "path": "/docs/database/validations", "label": "Validations & Callbacks" },
        { "path": "/docs/database/state-machines", "label": "State Machines" },
        { "path": "/docs/database/finders", "label": "Finders & Aggregations" },
        { "path": "/docs/database/analytics", "label": "Analytics & Columnar" },
        { "path": "/docs/database/search", "label": "Search: Vector & Geo" },
        { "path": "/docs/database/advanced", "label": "Advanced Features" },
        { "path": "/docs/database/migrations", "label": "Migrations" }
      ]
    },
    {
      "title": "Security",
      "icon": "<svg class=\"w-4 h-4 text-red-400\" fill=\"none\" viewBox=\"0 0 24 24\" stroke=\"currentColor\"><path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z\" /></svg>",
      "items": [
        { "path": "/docs/security/authentication", "label": "Authentication" },
        { "path": "/docs/security/authorization", "label": "Authorization" },
        { "path": "/docs/security/oidc-provider", "label": "OIDC Provider" },
        { "path": "/docs/security/oauth-client", "label": "OAuth Client" },
        { "path": "/docs/security/sessions", "label": "Sessions" },
        { "path": "/docs/security/defaults", "label": "Production defaults" },
        { "path": "/docs/builtins/validation", "label": "Validation" }
      ]
    },
    {
      "title": "Development Tools",
      "icon": "<svg class=\"w-4 h-4 text-orange-400\" fill=\"none\" viewBox=\"0 0 24 24\" stroke=\"currentColor\"><path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z\" /><path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"M15 12a3 3 0 11-6 0 3 3 0 016 0z\" /></svg>",
      "items": [
        { "path": "/docs/development-tools/live-reload", "label": "Live Reload" },
        { "path": "/docs/development-tools/static-server", "label": "Static & Markdown Server" },
        { "path": "/docs/development-tools/debugging", "label": "Debugging" },
        { "path": "/docs/development-tools/observability", "label": "Observability" },
        { "path": "/docs/development-tools/scaffold", "label": "Scaffold Generator" },
        { "path": "/docs/development-tools/editor-integration", "label": "Editor Integration" },
        { "path": "/docs/development-tools/formatting", "label": "Formatting" },
        { "path": "/docs/development-tools/deploy", "label": "Deploy" },
        { "path": "/docs/development-tools/desktop", "label": "Desktop Apps" },
        { "path": "/docs/development-tools/auto-update", "label": "Auto-Update (OTA)" },
        { "path": "/docs/development-tools/native-bridge", "label": "Native Bridge" },
        { "path": "/docs/native/notifications", "label": "Notifications", "indent": true },
        { "path": "/docs/native/camera", "label": "Camera & Microphone", "indent": true },
        { "path": "/docs/native/scanning", "label": "Barcode & QR Scanning", "indent": true },
        { "path": "/docs/native/geolocation", "label": "Geolocation", "indent": true },
        { "path": "/docs/native/motion-sensors", "label": "Motion Sensors", "indent": true },
        { "path": "/docs/native/device", "label": "Device Capabilities", "indent": true },
        { "path": "/docs/native/deep-links", "label": "Deep Links", "indent": true },
        { "path": "/docs/native/push-apple", "label": "Apple Push (APNs)", "indent": true },
        { "path": "/docs/native/push-android", "label": "Android Push (FCM)", "indent": true },
        { "path": "/docs/native/devices", "label": "Device registration", "indent": true },
        { "path": "/docs/native/clients", "label": "Native clients", "indent": true },
        { "path": "/docs/native/offline", "label": "Offline mobile", "indent": true },
        { "path": "/docs/native/platform-limits", "label": "Platform limits", "indent": true },
        { "path": "/docs/development-tools/ai-agents", "label": "AI Agents" },
        { "path": "/docs/development-tools/ai-evals", "label": "Agents on Soli" },
        { "path": "/docs/development-tools/linting", "label": "Linting" },
        { "path": "/docs/development-tools/graph", "label": "Code Graph" }
      ]
    },
    {
      "title": "Language Reference",
      "icon": "<svg class=\"w-4 h-4 text-orange-400\" fill=\"none\" viewBox=\"0 0 24 24\" stroke=\"currentColor\"><path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4\" /></svg>",
      "items": [
        { "path": "/docs/language", "label": "Overview" },
        { "path": "/docs/language/variables-types", "label": "Variables & Types" },
        { "path": "/docs/language/operators", "label": "Operators" },
        { "path": "/docs/language/control-flow", "label": "Control Flow" },
        { "path": "/docs/language/error-handling", "label": "Error Handling" },
        { "path": "/docs/language/functions", "label": "Functions" },
        { "path": "/docs/language/blocks", "label": "Blocks" },
        { "path": "/docs/language/integers", "label": "Integers" },
        { "path": "/docs/language/floats", "label": "Floats" },
        { "path": "/docs/language/booleans", "label": "Booleans" },
        { "path": "/docs/language/null", "label": "Null" },
        { "path": "/docs/language/decimal", "label": "Decimal" },
        { "path": "/docs/language/symbols", "label": "Symbols" },
        { "path": "/docs/language/strings", "label": "Strings" },
        { "path": "/docs/language/hashes", "label": "Hashes" },
        { "path": "/docs/language/arrays", "label": "Arrays" },
        { "path": "/docs/language/classes-oop", "label": "Classes & OOP" },
        { "path": "/docs/language/pattern-matching", "label": "Pattern Matching" },
        { "path": "/docs/language/enums", "label": "Enums" },
        { "path": "/docs/language/pipeline-operator", "label": "Pipeline Operator" },
        { "path": "/docs/language/modules", "label": "Modules" },
        { "path": "/docs/language/metaprogramming", "label": "Metaprogramming" }
      ]
    },
    {
      "title": "Built-in Functions",
      "icon": "<svg class=\"w-4 h-4 text-cyan-400\" fill=\"none\" viewBox=\"0 0 24 24\" stroke=\"currentColor\"><path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"M8 14v3m4-3v3m4-3v3M3 21h18M3 10h18M3 7l9-4 9 4M4 10h16v11H4V10z\" /></svg>",
      "items": [
        { "path": "/docs/builtins", "label": "Overview" },
        { "path": "/docs/builtins/core", "label": "Core" },
        { "path": "/docs/builtins/http", "label": "HTTP" },
        { "path": "/docs/builtins/s3", "label": "S3" },
        { "path": "/docs/builtins/soap", "label": "SOAP" },
        { "path": "/docs/builtins/pop3", "label": "POP3 (Email)" },
        { "path": "/docs/builtins/imap", "label": "IMAP (Email)" },
        { "path": "/docs/builtins/mailer", "label": "Mailer" },
        { "path": "/docs/builtins/file", "label": "File" },
        { "path": "/docs/utility/base64", "label": "Base64" },
        { "path": "/docs/utility/encoding", "label": "Encoding" },
        { "path": "/docs/builtins/spreadsheet", "label": "Spreadsheet" },
        { "path": "/docs/builtins/json", "label": "JSON" },
        { "path": "/docs/builtins/ai", "label": "AI" },
        { "path": "/docs/builtins/markdown", "label": "Markdown" },
        { "path": "/docs/builtins/crypto", "label": "Crypto / TOTP" },
        { "path": "/docs/builtins/jwt", "label": "JWT" },
        { "path": "/docs/builtins/paseto", "label": "PASETO" },
        { "path": "/docs/builtins/xml-signatures", "label": "XML Signatures & Keys" },
        { "path": "/docs/builtins/regex", "label": "Regex" },
        { "path": "/docs/builtins/env", "label": "Env" },
        { "path": "/docs/builtins/datetime", "label": "DateTime" },
        { "path": "/docs/builtins/duration", "label": "Duration" },
        { "path": "/docs/builtins/validation", "label": "Validation" },
        { "path": "/docs/builtins/session", "label": "Session" },
        { "path": "/docs/builtins/jobs", "label": "Jobs & Cron" },
        { "path": "/docs/builtins/testing", "label": "Testing" },
        { "path": "/docs/builtins/i18n", "label": "I18n" },
        { "path": "/docs/builtins/cache", "label": "Cache" },
        { "path": "/docs/builtins/kv", "label": "KV Store" },
        { "path": "/docs/builtins/rate-limit", "label": "Rate Limit" },
        { "path": "/docs/builtins/url", "label": "Url" },
        { "path": "/docs/builtins/logger", "label": "Logger" },
        { "path": "/docs/builtins/resilience", "label": "Retry & Circuit Breaker" },
        { "path": "/docs/builtins/toml-yaml", "label": "TOML / YAML" },
        { "path": "/docs/builtins/semaphore", "label": "Semaphore" },
        { "path": "/docs/builtins/money", "label": "Money" },
        { "path": "/docs/builtins/hardening", "label": "Hardening" },
        { "path": "/docs/builtins/security-headers", "label": "Security Headers" },
        { "path": "/docs/builtins/upload", "label": "Upload" },
        { "path": "/docs/builtins/vapid", "label": "VAPID" },
        { "path": "/docs/builtins/image", "label": "Image" },
        { "path": "/docs/builtins/pdf", "label": "PDF & Factur-X" },
        { "path": "/docs/builtins/pdf-templates", "label": "Invoice & Quote Templates" },
        { "path": "/docs/builtins/pdf-editor", "label": "Layout Editor" },
        { "path": "/docs/builtins/pdf-studio", "label": "PDF Studio" },
        { "path": "/docs/builtins/pdf-playground", "label": "PDF Playground" }
      ]
    },
    {
      "title": "Testing",
      "icon": "<svg class=\"w-4 h-4 text-orange-400\" fill=\"none\" viewBox=\"0 0 24 24\" stroke=\"currentColor\"><path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2\" /></svg>",
      "items": [
        { "path": "/docs/testing", "label": "Testing Guide" },
        { "path": "/docs/testing-browser", "label": "Browser Testing" },
        { "path": "/docs/testing-quick-reference", "label": "Quick Reference" }
      ]
    },
    {
      "title": "Blog",
      "icon": "<svg class=\"w-4 h-4 text-yellow-400\" fill=\"none\" viewBox=\"0 0 24 24\" stroke=\"currentColor\"><path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"M19 20H5a2 2 0 01-2-2V6a2 2 0 012-2h10a2 2 0 012 2v1m2 13a2 2 0 01-2-2V7m2 13a2 2 0 002-2V9a2 2 0 00-2-2h-2m-4-3H9M7 16h3M7 8h3m4 0h3\" /></svg>",
      "items": [
        { "path": "/docs/blog", "label": "Blog" }
      ]
    },
    {
      "title": "Internals",
      "icon": "<svg class=\"w-4 h-4 text-violet-400\" fill=\"none\" viewBox=\"0 0 24 24\" stroke=\"currentColor\"><path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4\" /></svg>",
      "items": [
        { "path": "/docs/internals", "label": "Rust crate map" },
        { "path": "/docs/internals/pipeline", "label": "Lexer / parser / AST" },
        { "path": "/docs/internals/interpreter", "label": "Interpreter" },
        { "path": "/docs/internals/vm", "label": "Bytecode VM" },
        { "path": "/docs/internals/serve", "label": "Serve / HTTP" },
        { "path": "/docs/internals/database", "label": "Database adapters" },
        { "path": "/docs/internals/rust-api", "label": "Types & methods" }
      ]
    }
  ]
end

# Every entry, flattened, in reading order — what the previous/next arrows walk.
#
# A page may legitimately be cross-listed: `/docs/builtins/validation` sits under
# both Security and Built-in Functions, because it is both. A sidebar can show
# it twice; a linear order cannot, since the page has one pair of arrows. The
# **first** occurrence wins, which keeps both sidebar links and makes the arrows
# deterministic rather than dependent on which occurrence a lookup happened to
# reach last.
def docs_nav_flat()
  flat = []
  seen = {}
  for section in docs_nav_sections()
    for item in section["items"]
      unless seen.has_key(item["path"])
        seen[item["path"]] = true
        flat.push(item)
      end
    end
  end
  flat
end

# The entries either side of `path`, as { "prev": …, "next": … }. Either may be
# null: the first page has no previous and the last has no next. A path the list
# does not contain gets neither, which is how a page outside the documentation
# renders no arrows rather than the wrong ones.
def docs_nav_neighbours(path: String) -> Hash
  flat = docs_nav_flat()
  idx  = -1
  i    = 0
  for item in flat
    if item["path"] == path
      idx = i
    end
    i = i + 1
  end

  return { "prev": null, "next": null } if idx < 0

  {
    "prev": idx > 0 ? flat[idx - 1] : null,
    "next": idx < flat.length() - 1 ? flat[idx + 1] : null
  }
end
