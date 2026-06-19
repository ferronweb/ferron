# Ferron 3 documentation

User-facing documentation for Ferron 3. Synced to the documentation website on pushes to `3.x`.

## Structure

```
docs/
├── index.md                       # Landing page
├── getting-started.md             # First-time user guide
├── troubleshooting.md             # Diagnostic checklist
├── installation/                  # Platform-specific install guides
│   ├── linux/
│   ├── docker.md
│   ├── windows.md
│   ├── manual-installation.md
│   └── source/
├── migration/
│   └── from-v2.md                 # Ferron 2 → 3 migration
├── use-cases/                     # Task-oriented feature guides
│   ├── content/                   # static files, caching, CGI, PHP
│   ├── traffic/                   # reverse proxy, URL rewriting, error pages
│   ├── security/                  # TLS, rate limiting, abuse, headers, mTLS, access control
│   └── operations/                # admin API, logging, ferron-serve
├── configuration/                 # Directive-level reference
│   ├── fundamentals/              # syntax, JSON, validation, doctor, conditionals
│   ├── server/                    # core directives, host directives
│   ├── routing/                   # URL processing, response control, rewrite, map
│   ├── proxy/                     # reverse proxy, forward proxy
│   ├── security/                  # auth, TLS, ACME, DNS, OCSP, session tickets
│   ├── content/                   # static files, compression, cache, CGI, FastCGI, SCGI, buffering, headers, rate limit, abuse
│   └── observability/             # logging, metrics, tracing, OTLP, Prometheus
├── docLinks.ts                    # Sidebar link definitions (internal use by the website)
└── README.md                      # This file
```

Two tiers of documentation:

- **Use-case guides** (`use-cases/`) — task-oriented walkthroughs that show how to accomplish a goal (e.g., "set up automatic TLS").
- **Configuration reference** (`configuration/`) — exhaustive directive-level pages organized by functional area. Expects the reader to already know what they're looking for.

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

Sentence case. Use `##` for top-level section headings, `###` for subsections. No trailing `## Notes and troubleshooting` section — use inline callouts instead.

### Code blocks

Use ` ```ferron ` for configuration examples. Use ` ``` ` (no language tag) for shell commands or other output.

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

| Alert type | Usage |
|------------|-------|
| `> [!tip]` | Best practices, shortcuts, recommendations |
| `> [!note]` | Neutral clarification or supplementary detail |
| `> [!important]` | Critical requirement or consequence |
| `> [!warning]` | Potential pitfall or configuration risk |
| `> [!info]` | Cross-reference to related documentation |

### Links

Use relative paths prefixed with `/docs/v3/`, without `.md` file extension:

```markdown
See [Reverse proxying](/docs/v3/use-cases/traffic/reverse-proxy).
```

### Writing principles

- **Describe behavior, not labels** — explain what the system actually does, not just what the feature is called.
- **Functional precision first** — prefer clear, explicit descriptions over clever phrasing.
- **Consistency over novelty** — if a term comes from an upstream API, a legacy config, or a widely adopted standard, keep it.
- **Inline callouts** — no separate notes section at the end of a page.
- **No emojis** unless the content explicitly calls for them.
- **Linters are guidance** — don't let woke or other terminology linters override clarity or consistency.

## Sidebar

The sidebar navigation is defined in `docLinks.ts`. Add new pages there to make them discoverable on the documentation website.

**Fields:**

| Name | Description |
|------|-------------|
| `href` | The URL path of the page (e.g., `/docs/v3/installation/linux/rhel-fedora`) |
| `target` | The target window or tab (`"_self"` for current, `"_blank"` for new) |
| `label` | The display text in the sidebar |
| `sub` | Whether this is a sub-item (indented under another category) |
| `category` | Whether this item should be treated as a category header only |
