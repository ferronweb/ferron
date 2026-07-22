# Ferron documentation

This directory contains the documentation for the Ferron web server. If you're looking for the server documentation, you can go to <https://ferron.sh/docs>.

## `links.json` file

The `links.json` file contains a list of links to the documentation pages. The list is in this format:

```json
[
  {
    "href": "/docs", // Destination path
    "target": "_self", // Target (for example, "_self" or "_blank")
    "sub": false, // Whether the link is a subpage
    "label": "Welcome to the documentation!", // Link text
  },
  // ...
]
```
