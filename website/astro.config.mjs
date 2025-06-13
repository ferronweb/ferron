// @ts-check
import { defineConfig } from "astro/config";

import tailwindcss from "@tailwindcss/vite";
import react from "@astrojs/react";
import sitemap from "@astrojs/sitemap";

import kdl from "./kdl.tmLanguage.json";

// https://astro.build/config
export default defineConfig({
  site: "https://www.ferronweb.org",

  vite: {
    plugins: [tailwindcss()],
    ssr: {
      noExternal: ["@fontsource/ibm-plex-sans"]
    },
    build: {
      assetsInlineLimit: 0,
      chunkSizeWarningLimit: 600
    }
  },
  integrations: [react(), sitemap()],
  markdown: {
    shikiConfig: {
      theme: "nord",
      langs: [kdl]
    }
  },
  prefetch: {
    prefetchAll: true,
    defaultStrategy: "tap"
  }
});
