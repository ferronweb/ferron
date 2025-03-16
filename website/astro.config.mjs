// @ts-check
import { defineConfig } from "astro/config";

import tailwindcss from "@tailwindcss/vite";
import react from "@astrojs/react";

import sitemap from "@astrojs/sitemap";

// https://astro.build/config
export default defineConfig({
  site: "https://www.ferronweb.org",

  vite: {
    plugins: [tailwindcss()],
    ssr: {
      noExternal: ["@fontsource/inter", "@fontsource/rajdhani"]
    },
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
    }
  }
});
