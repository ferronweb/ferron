# Ferron documentation

This directory contains the documentation for the Ferron web server. If you're looking for the server documentation, you can go to <https://ferron.sh/docs>.

## `docLinks.ts` file

The `docLinks.ts` file contains a list of links to the documentation pages. The list is in this format:

```typescript
export default [
  {
    href: "/docs", // Destination path
    target: "_self", // Target (for example, "_self" or "_blank")
    sub: false, // Whether the link is a subpage
    label: "Welcome to the documentation!", // Link text
  },
  // ...
];
```
