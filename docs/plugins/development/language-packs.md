# Creating Language Packs

Language packs add syntax highlighting, language configuration, and LSP support for new languages in Fresh.

## Quick Start

Use the CLI to scaffold a new language pack:

```bash
fresh --init language
```

This creates a directory with the basic structure:
```
my-language/
├── package.json          # Package manifest
├── grammars/
│   └── syntax.sublime-syntax  # Sublime syntax grammar (YAML)
├── validate.sh           # Validation script
└── README.md
```

## Package Structure

### package.json

The manifest configures the language pack:

```json
{
  "$schema": "https://raw.githubusercontent.com/sinelaw/fresh/main/crates/fresh-editor/plugins/schemas/package.schema.json",
  "name": "my-language",
  "version": "0.1.0",
  "description": "Language support for MyLang",
  "type": "language",
  "author": "Your Name",
  "license": "MIT",
  "fresh": {
    "grammar": {
      "file": "grammars/syntax.sublime-syntax",
      "extensions": ["mylang", "ml"]
    },
    "language": {
      "commentPrefix": "//",
      "blockCommentStart": "/*",
      "blockCommentEnd": "*/",
      "tabSize": 4,
      "autoIndent": true
    },
    "lsp": {
      "command": "my-language-server",
      "args": ["--stdio"],
      "autoStart": true
    }
  }
}
```

### Grammar Configuration

| Field | Description |
|-------|-------------|
| `file` | Path to the grammar file (relative to package root) |
| `extensions` | File extensions this grammar handles (without dots) |
| `firstLine` | Optional regex for shebang detection |

### Language Configuration

| Field | Description |
|-------|-------------|
| `commentPrefix` | Line comment prefix (e.g., `//`, `#`, `--`) |
| `blockCommentStart` | Block comment opening (e.g., `/*`, `<!--`) |
| `blockCommentEnd` | Block comment closing (e.g., `*/`, `-->`) |
| `tabSize` | Default indentation width |
| `useTabs` | Use tabs instead of spaces |
| `autoIndent` | Enable automatic indentation |
| `formatter.command` | Formatter command (e.g., `prettier`) |
| `formatter.args` | Arguments for the formatter |

### LSP Configuration

| Field | Description |
|-------|-------------|
| `command` | LSP server executable |
| `args` | Arguments to pass to the server |
| `autoStart` | Start server when opening matching files |
| `initializationOptions` | Custom LSP initialization options |

## Finding Existing Grammars

Before writing a grammar from scratch, search online for existing Sublime Text or TextMate grammars:

1. **Search GitHub** for `<language> sublime-syntax` or `<language> tmLanguage`
2. **Check VS Code extensions** - many use TextMate/Sublime grammars
3. **Browse [Package Control](https://packagecontrol.io/)** - Sublime Text's package repository

When using an existing grammar:

1. **Check the license** - ensure it allows redistribution (MIT, Apache, BSD are common)
2. **Include a copy of the license** in your `grammars/` directory (e.g., `grammars/LICENSE`)
3. **Credit the original author** in your README and package description

Example attribution in README:
```markdown
## Grammar Attribution

The syntax grammar is derived from [original-package](https://github.com/user/repo)
by Original Author, licensed under MIT. See `grammars/LICENSE` for details.
```

## Writing Sublime Syntax Grammars

Fresh uses Sublime Text's `.sublime-syntax` format (YAML-based). This is the recommended format because:
- More readable than JSON TextMate grammars
- Better tooling and documentation
- Supports advanced features like contexts and includes

### Basic Structure

```yaml
%YAML 1.2
---
name: My Language
scope: source.mylang
file_extensions: [mylang, ml]

contexts:
  main:
    - include: comments
    - include: strings
    - include: keywords
    - include: numbers

  comments:
    - match: //.*$
      scope: comment.line.double-slash

    - match: /\*
      scope: punctuation.definition.comment.begin
      push:
        - meta_scope: comment.block
        - match: \*/
          scope: punctuation.definition.comment.end
          pop: true

  strings:
    - match: '"'
      scope: punctuation.definition.string.begin
      push:
        - meta_scope: string.quoted.double
        - match: \\.
          scope: constant.character.escape
        - match: '"'
          scope: punctuation.definition.string.end
          pop: true

  keywords:
    - match: \b(if|else|while|for|return|fn|let|const)\b
      scope: keyword.control

  numbers:
    - match: \b[0-9]+(\.[0-9]+)?\b
      scope: constant.numeric
```

### Key Concepts

**Scopes**: Define how text is styled. Common scopes include:
- `comment.line`, `comment.block` - Comments
- `string.quoted.single`, `string.quoted.double` - Strings
- `keyword.control`, `keyword.operator` - Keywords
- `constant.numeric`, `constant.language` - Constants
- `entity.name.function`, `entity.name.class` - Declarations
- `variable.parameter`, `variable.other` - Variables

**Contexts**: Named states for the parser. Use `push`/`pop` for nested structures.

**Includes**: Reuse rules across contexts with `include`.

### Resources

- [Sublime Text Syntax Documentation](https://www.sublimetext.com/docs/syntax.html)
- [Scope Naming Conventions](https://www.sublimetext.com/docs/scope_naming.html)
- [TextMate Grammar Reference](https://macromates.com/manual/en/language_grammars)

## Examples

### Minimal Example

See the [Solidity language pack](https://github.com/sinelaw/fresh-plugins/tree/main/languages/solidity):

```
languages/solidity/
├── package.json
├── grammars/
│   ├── solidity.sublime-syntax
│   └── LICENSE
├── validate.sh
└── README.md
```

### Grammar with Embedded Languages

See the [Templ language pack](https://github.com/sinelaw/fresh-plugins/tree/main/languages/templ) for an example that extends Go syntax:

```yaml
%YAML 1.2
---
name: Templ
scope: source.go.templ
extends: Packages/Go/Go.sublime-syntax

file_extensions:
  - templ

contexts:
  # Custom rules that extend the base Go grammar
```

## Testing

1. **Local testing**: Copy your language pack to `~/.config/fresh/grammars/`
2. **Validate manifest**: Run `./validate.sh` in your package directory
3. **Restart Fresh** to load the new grammar

## Publishing

1. Push your package to a public Git repository
2. Submit a PR to [fresh-plugins-registry](https://github.com/sinelaw/fresh-plugins-registry)
3. Add your package to `languages.json`

After approval, users can install with:
```
:pkg install your-language
```
