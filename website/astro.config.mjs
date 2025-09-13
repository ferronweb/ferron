// @ts-check
import { defineConfig, fontProviders } from "astro/config";

import tailwindcss from "@tailwindcss/vite";
import react from "@astrojs/react";
import sitemap from "@astrojs/sitemap";

import rehypeWrap from "rehype-wrap";

// https://astro.build/config
export default defineConfig({
  site: "https://www.ferronweb.org",

  vite: {
    plugins: [tailwindcss()],
    build: {
      assetsInlineLimit: 0,
      chunkSizeWarningLimit: 600
    },
    optimizeDeps: { include: ["asciinema-player"] }
  },
  integrations: [react(), sitemap()],
  markdown: {
    shikiConfig: {
      theme: "nord"
    },
    rehypePlugins: [
      [rehypeWrap, { wrapper: "div.overflow-x-auto", selector: "table" }]
    ]
  },
  prefetch: {
    prefetchAll: true,
    defaultStrategy: "tap"
  },
  experimental: {
    fonts: [
      {
        provider: fontProviders.fontsource(),
        name: "Rajdhani",
        weights: [400, 700],
        cssVariable: "--font-rajdhani",
        fallbacks: ["sans-serif"]
      },
      {
        provider: fontProviders.fontsource(),
        name: "Inter",
        weights: [400, 700],
        cssVariable: "--font-inter",
        fallbacks: ["sans-serif"]
      }
    ]
  }
});
