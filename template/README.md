# app_name

A Soli MVC application.

## Getting Started

### Development Server

Start the development server with hot reload:

```bash
soli serve . --dev
```

Your app will be available at [http://localhost:5011](http://localhost:5011)

### Production Server

Start the production server:

```bash
soli serve . --port 3000
```

Or run as a daemon:

```bash
soli serve . -d
```

## Project Structure

```
app_name/
├── app/
│   ├── controllers/     # Request handlers
│   └── views/           # HTML templates
│       ├── home/        # Home page views
│       └── layouts/     # Layout templates
├── config/
│   └── routes.sl        # Route definitions
├── public/
│   └── css/             # Stylesheets
├── stdlib/              # Standard library modules
│   └── state_machine.sl # State machine implementation
├── tests/               # Test files
├── package.json         # NPM dependencies
└── tailwind.config.js   # TailwindCSS configuration
```

## Standard Library

This template includes the `stdlib/` folder with useful modules:

- **state_machine.sl** - State machine implementation for managing complex workflows

```soli
import { create_state_machine } from "./stdlib/state_machine.sl";

let order = create_state_machine("pending",
    ["pending", "confirmed", "processing", "shipped", "delivered"],
    [
        {"event": "confirm", "from": "pending", "to": "confirmed"},
        {"event": "process", "from": "confirmed", "to": "processing"}
    ]
);

order.transition("confirm");
print(order.current_state());  // "confirmed"
```

## Building CSS

Install dependencies and build TailwindCSS:

```bash
npm install
npm run build:css
```

For development with hot reload:

```bash
npm run watch:css
```

## Learn More

- [Soli Documentation](http://localhost:5011/docs)
- [State Machines Guide](http://localhost:5011/docs/core-concepts/state-machines)
