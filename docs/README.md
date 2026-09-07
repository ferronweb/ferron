# Ferron 3 documentation

User-facing documentation for Ferron 3. Synced to the documentation website on pushes to `3.x`.

## Structure

Two tiers of documentation:

- **Use-case guides** (`use-cases/`): task-oriented walkthroughs that show how to accomplish a goal (for example, "set up automatic TLS").
- **Configuration reference** (`configuration/`): exhaustive directive-level pages organized by functional area. Expects the reader to already know what they need.

The `getting-started.md` and `index.md` pages bridge the two tiers with recommended reading paths.

## Style guide

### Frontmatter

Every page has YAML frontmatter with `title` and `description`.

```yaml
---
title: "Page title"
description: "One- or two-sentence summary of what this page covers."
---
```

### Headings

Sentence case. Use `##` for top-level section headings, `###` for subsections. No trailing `## Notes and troubleshooting` section, use inline callouts instead.

### Code blocks

Use ` ```ferron ` for configuration examples. Use ` ```bash ` or ` ```sh ` for shell commands. Use ` ```text ` for plain output.

Config files use `.conf` or `.ferron` extensions.

### Configuration style

Write `.conf` examples in idiomatic Ferron 3 style:

- **No semicolons**: end directives with newlines, not semicolons.
- **4-space indentation**: use 4 spaces at each nesting level.
- **Bare strings preferred**: omit quotes unless the value contains spaces, special characters, or ambiguity.
- **Boolean flags**: write the bare directive to enable it. Write `directive false` only to disable it.
- **Raw string literals**: use `r"..."` for regex patterns to avoid double-backslash escaping.
- **Quoted strings**: single and double quotes are interchangeable. Use the clearer option.

### Invalid configuration examples

Prefix the first line with `# INVALID` followed by a brief explanation:

```ferron
# INVALID: bogus TLS provider
example.com {
    tls {
        provider bogus
    }
}
```

### Callouts

Use GFM alert syntax inline with the relevant content. Do not gather callouts into a separate section at the end of the page.

| Alert type       | Usage                                         |
| ---------------- | --------------------------------------------- |
| `> [!tip]`       | Best practices, shortcuts, recommendations    |
| `> [!note]`      | Neutral clarification or supplementary detail |
| `> [!important]` | Critical requirement or consequence           |
| `> [!warning]`   | Potential pitfall or configuration risk       |
| `> [!info]`      | Cross-reference to related documentation      |

### Links

Use relative paths prefixed with `/docs/`, without `.md` file extension:

```markdown
See [Reverse proxying](/docs/use-cases/traffic/reverse-proxy).
```

### Writing principles

- **Describe behavior, not labels**: explain what the system actually does, not just what the feature is called.
- **Documentation scope**: treat the documentation as a user-facing manual. Do not include internal implementation details. Do not quote specifications directly.
- **Short paragraphs**: write short paragraphs. Keep each paragraph easy to scan. Cover one topic per paragraph with a maximum of six sentences.
- **Simple English**: write in clear, straightforward language. Use [ASD-STE100](https://asd-ste100.org/) as a reference for simplified English.
- **Functional precision first**: prefer clear, explicit descriptions over clever phrasing.
- **Consistency over novelty**: if a term comes from an upstream API, a legacy config, or a widely adopted standard, keep it.
- **Inline callouts**: no separate notes section at the end of a page.
- **No emojis** unless the content explicitly calls for them.
- **Linters are guidance**: do not let `woke` or other terminology linters override clarity or consistency.
- **STE rules for docs prose**: Apply these Simplified Technical English rules to all documentation (headings, paragraphs, list items, callout text — not code blocks, inline code, directive names, or URLs):
  - **Active voice**: "Ferron reads the file", not "the file is read by the parser". Use "Ferron" or "the server" as the subject when the actor is the software.
  - **No contractions**: Write "do not", "cannot", "will not", "it is".
  - **Sentence length**: Max 20 words for instructions, max 25 for descriptive sentences. Split longer sentences.
  - **No semicolons**: Use a period and split into two sentences.
  - **Replace banned words**: begin→start, ensure→make sure, utilize→use, "prior to"→before, "subsequent to"→after, obtain→get, demonstrate→show, additionally→also, "in order to"→to, "a variety of"→various, "it is important to note"→delete/restate, "due to the fact that"→because.
  - **No marketing adjectives**: seamless, robust, powerful, effortless, and so on.
  - **No nominalizations**: "perform an analysis"→"analyze", "provide documentation"→"document".
  - **No "-ing" main verbs**: "is creating"→"creates", "is running"→"runs".
  - **One topic per paragraph**, max six sentences per paragraph.
  - **No em dashes as separators**: Use a period, comma, or restructure instead. Keep numeric ranges (1-2).
  - **American spelling**.

## Validation

Run these commands from the repository root after you change configuration docs:

```bash
cargo run -p ferron -- validate -c ferron.conf
cargo run --manifest-path doctest/Cargo.toml
```

Ferron validates the sample config. The doctest harness runs doc examples against the built binary.

## Sidebar

Define the sidebar navigation in `links.json`. Add new pages there to make them discoverable on the documentation website.

**Fields:**

| Name       | Description                                                                    |
| ---------- | ------------------------------------------------------------------------------ |
| `href`     | The URL path of the page (for example, `/docs/installation/linux/rhel-fedora`) |
| `target`   | The target window or tab (`"_self"` for current, `"_blank"` for new)           |
| `label`    | The display text in the sidebar                                                |
| `sub`      | Whether this is a sub-item (indented under another category)                   |
| `category` | Whether the item should function as a category header only                     |
