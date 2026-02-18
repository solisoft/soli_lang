# Installation

## Prerequisites

- Node.js (v16 or higher)
- npm or yarn

## Install SoliLang

### Quick Install (Recommended)

```bash
curl -sSL https://raw.githubusercontent.com/solisoft/soli_lang/main/install.sh | sh
```

This detects your OS and architecture, downloads the latest release binary, and installs it to `~/.local/bin`.

For system-wide installation:

```bash
curl -sSL https://raw.githubusercontent.com/solisoft/soli_lang/main/install.sh | sh -s -- --system
```

### Via Cargo

```bash
cargo install solilang
```

### From Source

```bash
# Clone the repository
git clone https://github.com/solisoft/soli_lang.git
cd soli_lang

# Build the project
cargo build --release

# Install globally
cargo install --path .
```

## Create a New MVC Project

```bash
# Clone this example or template
git clone https://github.com/solilang/solilang.git
cd solilang/examples/mvc_app

# Install frontend dependencies
npm install

# Build CSS
npm run build:css

# Start development server
npm run dev
```

## Project Setup

### 1. Configure Routes

Edit `config/routes.sl`:

```soli
get("/", "home#index");
get("/about", "home#about");
post("/contact", "home#contact");
```

### 2. Create Controllers

Create controllers in `app/controllers/`:

```soli
fn index(req) {
    return render("home/index", {
        "title": "Welcome"
    });
}
```

### 3. Add Views

Create templates in `app/views/home/`:

```erb
<h1><%= title %></h1>
<p>Welcome to my app!</p>
```

## Running in Development

```bash
# Start both Tailwind watcher and Soli server
npm run dev
```

This starts both the SoliLang server and the TailwindCSS watcher with hot reload.

## Building for Production

```bash
# Build CSS
npm run build:css

# Build Soli application
cargo build --release
```

## Verifying Installation

Create a test file:

```soli
// test.sl
println("Hello, SoliLang!");
```

Run it:

```bash
soli test.sl
```

You should see: `Hello, SoliLang!`
