# Soli Language Support for VSCode

This extension provides syntax highlighting for the Soli programming language in VSCode and other editors that support TextMate grammars.

## Installation

### From Source

1. Navigate to this directory:
   ```bash
   cd editors/vscode
   ```

2. Install VSCE (VSCode Extension Publisher) if not already installed:
   ```bash
   npm install -g vsce
   ```

3. Package the extension:
   ```bash
   vsce package
   ```

4. Install the generated `.vsix` file in VSCode:
   - Open VSCode
   - Press `Ctrl+Shift+P` (or `Cmd+Shift+P` on Mac)
   - Type "Extensions: Install from VSIX"
   - Select the generated `.vsix` file

### Manual Installation

Copy the `syntaxes/` folder and `language-configuration.json` to your VSCode extensions folder:

- **Windows**: `%USERPROFILE%\.vscode\extensions\soli-language\`
- **macOS**: `~/.vscode/extensions/soli-language/`
- **Linux**: `~/.vscode/extensions/soli-language/`

## Features

- Syntax highlighting for Soli source files (`.sl`)
- Support for:
  - Keywords (let, fn, if, else, class, etc.)
  - Types (Int, Float, Bool, String, Void)
  - String literals (including escape sequences)
  - Numbers (decimal, hex, binary, octal)
  - Comments (single-line and multi-line)
  - Operators and punctuation

## Language Support

This extension recognizes the following Soli language features:

### Keywords
- Control flow: `if`, `else`, `elsif`, `while`, `for`, `in`, `match`, `case`, `when`, `end`, `unless`
- Error handling: `try`, `catch`, `finally`, `throw`
- Functions: `fn`, `return`
- Variables: `let`
- Classes: `class`, `extends`, `implements`, `interface`, `new`, `this`, `super`
- Visibility: `public`, `private`, `protected`, `static`
- Modules: `import`, `export`, `from`, `as`
- Async: `async`, `await`

### Types
- `Int`, `Float`, `Bool`, `String`, `Void`, `Any`

### Built-in Values
- `true`, `false`, `null`

## File Extension

Files with the `.sl` extension are recognized as Soli source files.
