---
title: "Editor support"
description: "Syntax highlighting, language server, and editor extensions for Ferron configuration files."
---

Ferron configuration files (`.conf` / `.ferron`) are supported by a range of editor integrations. These cover syntax highlighting, formatting, completions, and diagnostics; useful for GitOps operators and anyone editing Ferron configuration by hand.

## Feature overview

| Feature                   | Description                                                                            |
| ------------------------- | -------------------------------------------------------------------------------------- |
| **Syntax highlighting**   | TextMate grammar and tree-sitter grammar for `.conf` / `.ferron` files                 |
| **Language server (LSP)** | `ferron-language-server` — provides formatting, directive completions, and diagnostics |
| **VS Code extension**     | Official extension bundling syntax highlighting and LSP                                |
| **Zed extension**         | Official extension for Zed (submission under review)                                   |
| **Neovim**                | Manual configuration using tree-sitter and LSP                                         |

## Syntax highlighting

Syntax highlighting is provided by two independent grammars. Use the one compatible with your editor.

### TextMate grammar

The TextMate grammar is hosted at [ferronweb/ferronconf](https://github.com/ferronweb/ferronconf) and distributed as `ferron.tmLanguage.json`:

```text
https://raw.githubusercontent.com/ferronweb/ferronconf/refs/heads/main/ferron.tmLanguage.json
```

This grammar works with any editor that accepts TextMate grammars, including:

- **Visual Studio Code** and forks (via the VS Code extension below, or manual grammar installation)
- **Sublime Text**
- **Atom**
- **BBEdit**

### tree-sitter grammar

The tree-sitter grammar is hosted at [ferronweb/tree-sitter-ferron](https://github.com/ferronweb/tree-sitter-ferron) and published as `tree-sitter-ferron` on [npm](https://www.npmjs.com/package/tree-sitter-ferron) and [crates.io](https://crates.io/crates/tree-sitter-ferron).

This grammar works with editors that use tree-sitter, including:

- **Neovim** (via `nvim-treesitter`)
- **Zed** (via the Zed extension below, or manual grammar installation)
- **Helix**
- **Emacs** (via `tree-sitter`)

## Language server (LSP)

The Ferron language server provides:

- **Formatting** — formats `.conf` files using the same rules as `ferron-fmt`
- **Directive completions** — suggests valid directives based on the loaded module set and current block context
- **Diagnostics** — reports parse errors, unknown directives, and invalid configurations, based on the same validation logic as `ferron validate`

### Installation

The language server is published as `ferron-language-server` on [npm](https://www.npmjs.com/package/ferron-language-server) and [crates.io](https://crates.io/crates/ferron-language-server).

```bash
npm install -g ferron-language-server
```

Prebuilt binaries are also available from [GitHub releases](https://github.com/ferronweb/ferron-language-server/releases) and can be downloaded from [dl.ferron.sh](https://dl.ferron.sh/ferron-language-server).

### Configuration

The language server accepts the following options:

| Option     | Default | Description                                                                                                              |
| ---------- | ------- | ------------------------------------------------------------------------------------------------------------------------ |
| `--ferron` | —       | Path to the directory containing the `ferron` binary (used for validation) and `ferron-fmt` binary (used for formatting) |

## VS Code

The official VS Code extension is published as **Ferron** by `Ferron.ferron`:

- [Visual Studio Marketplace](https://marketplace.visualstudio.com/items?itemName=Ferron.ferron)
- [Open VSX Registry](https://open-vsx.org/extension/Ferron/ferron)

The extension bundles both the TextMate grammar and the language server, so no separate installation is needed.

### Manual setup

If you prefer not to use the extension, you can install the TextMate grammar manually:

1. Download the grammar file:

   ```bash
   curl -L https://raw.githubusercontent.com/ferronweb/ferronconf/refs/heads/main/ferron.tmLanguage.json \
     -o ~/.vscode/extensions/ferron.tmLanguage.json
   ```

2. Install the language server separately (see [Installation](#installation) above) and configure it in your editor's LSP settings.

## Zed

The official Zed extension is hosted at [ferronweb/ferron-zed](https://github.com/ferronweb/ferron-zed). It is not yet published to the Zed extension registry (the submission is under review), but it can be installed manually:

1. Clone the extension:

   ```bash
   git clone https://github.com/ferronweb/ferron-zed.git
   cd ferron-zed
   ```

2. Install it as a development extension ("Install Dev Extension" button in Extensions)
3. Restart Zed.

The extension bundles the tree-sitter grammar and configures the language server automatically.

## Other editors

Any editor that supports TextMate grammars or tree-sitter can use Ferron syntax highlighting. Editors that support the Language Server Protocol can use the Ferron language server for completions and diagnostics.

For editors without a dedicated extension, configure them to:

1. Associate `.conf` or `.ferron` files with the Ferron language
2. Install the TextMate or tree-sitter grammar
3. Point the LSP client to the `ferron-language-server` binary

## See also

- [Syntax and file structure](/docs/v3/configuration/fundamentals/syntax)
- [Configuration formatting](/docs/v3/configuration/fundamentals/formatting) (`ferron-fmt` for formatting `.conf` files)
- [Configuration validation](/docs/v3/configuration/fundamentals/validation)
- [Configuration doctor](/docs/v3/configuration/fundamentals/doctor)
