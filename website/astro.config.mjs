// @ts-check

import sitemap from "@astrojs/sitemap";
import svelte from "@astrojs/svelte";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig, fontProviders } from "astro/config";
import minify from "astro-minify-html-swc";
import pagefind from "astro-pagefind";
import rehypeWrap from "rehype-wrap";
import { visit } from "unist-util-visit";
import kdl from "./kdl.tmLanguage.json";

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
  integrations: [svelte(), sitemap(), pagefind(), minify()],
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
