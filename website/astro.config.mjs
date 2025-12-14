// @ts-check
import { defineConfig, fontProviders } from "astro/config";

import tailwindcss from "@tailwindcss/vite";
import sitemap from "@astrojs/sitemap";
import pagefind from "astro-pagefind";

import kdl from "./kdl.tmLanguage.json";

import rehypeWrap from "rehype-wrap";
import { visit } from "unist-util-visit";

// https://astro.build/config
export default defineConfig({
  site: "https://ferron.sh",

  vite: {
    plugins: [tailwindcss()],
    build: {
      assetsInlineLimit: 0,
      chunkSizeWarningLimit: 600
    }
  },
  integrations: [
    sitemap(),
    pagefind(),
    (await import("astro-compress")).default({
      HTML: true, // This setting wouldn't work with React (it would cause hydration errors), but since the website uses vanilla JS, it's safe to enable.
      CSS: false // When enabled, it doesn't work with `rolldown-vite` (broken responsive styles)
    })
  ],
  markdown: {
    shikiConfig: {
      themes: {
        light: "catppuccin-latte",
        dark: "catppuccin-mocha"
      },
      langs: [kdl],
      defaultColor: false
    },
    remarkPlugins: [
      function remarkNofollowLinks() {
        return (tree, file) => {
          const data = file.data.astro?.frontmatter || {};
          if (!data.nofollow) return;

          // Traverse Markdown AST
          visit(tree, "link", (node) => {
            // Add rel="nofollow" via data.hProperties (used by rehype)
            node.data ??= {};
            node.data.hProperties ??= {};
            node.data.hProperties.rel = "nofollow";
          });
        };
      }
    ],
    rehypePlugins: [
      [rehypeWrap, { wrapper: "div.overflow-x-auto", selector: "table" }]
    ]
  },
  prefetch: {
    prefetchAll: true,
    defaultStrategy: "tap"
  },
  build: {
    format: "preserve"
  },
  trailingSlash: "never",
  experimental: {
    fonts: [
      {
        provider: fontProviders.fontsource(),
        name: "Funnel Sans",
        weights: [400, 500, 600, 700, 800],
        cssVariable: "--font-funnel-sans",
        fallbacks: ["Tahoma", "Arial", "Helvetica", "sans-serif"],
        subsets: ["latin", "latin-ext"]
      },
      {
        provider: fontProviders.fontsource(),
        name: "JetBrains Mono",
        weights: [400, 600],
        cssVariable: "--font-jetbrains-mono",
        fallbacks: ["monospace"],
        subsets: ["latin", "latin-ext"]
      }
    ]
  }
});
