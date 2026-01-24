# Introduction to SoliLang MVC

The SoliLang MVC Framework provides a clean, locally compiled, and organized structure for building web applications.

## Features

### Type-Safe
Built with Solilang's type system for compile-time safety and runtime reliability.

### Hot Reload
Instant updates during local development. Edit a file and see changes immediately.

### Fast Compilation
Native Rust compilation means your applications start instantly and run with minimal overhead.

## Architecture

The MVC pattern separates your application into three main layers:

- **Model**: Manages data and business logic.
- **View**: Handles presentation and HTML rendering.
- **Controller**: Orchestrates the flow between models and views.

## Quick Start

```bash
# Clone the MVC template
git clone https://github.com/solilang/mvc-template myapp
cd myapp

# Install dependencies
npm install

# Start development server
./dev.sh
```

## Directory Structure

```
mvc_app/
├── app/
│   ├── controllers/    # Request handlers
│   ├── middleware/     # HTTP middleware
│   ├── models/         # Data models
│   └── views/          # Templates
├── config/
│   └── routes.sl     # Route definitions
├── public/             # Static assets
└── views/
    └── layouts/        # Page layouts
```

## Soli Language

Soli is the programming language that powers the MVC framework. It features:

- **Static typing** with type inference
- **Classes and inheritance** for OOP
- **Interfaces** for contracts
- **Pattern matching** for elegant conditionals
- **Pipeline operator** for readable data transformation

### Learn the Soli Language

The Soli language has its own comprehensive documentation:

- **[Soli Language Reference](/docs/soli-language)** - Complete guide to Soli syntax, types, functions, classes, and more
- [Official Soli Documentation](https://solilang.com/docs/guides/introduction) - Full language documentation

## Design Philosophy

Soli favors convention over configuration. By following standard naming patterns, you write less glue code and focus on building features.

## Guides

- **[Scaffold Generator](/docs/scaffold)** - Quickly generate complete MVC resources
- **[Authentication](/docs/authentication)** - JWT-based stateless authentication
- **[Sessions](/docs/sessions)** - Cookie-based session management
- **[Validation](/docs/validation)** - Schema-based input validation
- **[Request Parameters](/docs/request-params)** - Unified access to route, query, and body parameters
- **[Testing](/docs/testing)** - BDD testing framework with coverage reporting
- **[Models](/docs/models)** - ORM-style data modeling with CRUD operations
- **[Migrations](/docs/migrations)** - Database schema versioning and management
