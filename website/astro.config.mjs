// @ts-check
import { defineConfig, fontProviders } from "astro/config";

import tailwindcss from "@tailwindcss/vite";
import react from "@astrojs/react";
import sitemap from "@astrojs/sitemap";
import pagefind from "astro-pagefind";

import kdl from "./kdl.tmLanguage.json";

// https://astro.build/config
export default defineConfig({
  site: "https://v2.ferronweb.org",

  vite: {
    plugins: [tailwindcss()],
    build: {
      assetsInlineLimit: 0,
      chunkSizeWarningLimit: 600
    }
  },
  integrations: [
    react(),
    sitemap(),
    pagefind(),
    (await import("astro-compress")).default({
      HTML: false
    })
  ],
  markdown: {
    shikiConfig: {
      theme: "nord",
      langs: [kdl]
    }
  },
  prefetch: {
    prefetchAll: true,
    defaultStrategy: "tap"
  },
  experimental: {
    fonts: [
      {
        provider: fontProviders.fontsource(),
        name: "IBM Plex Sans",
        weights: [400, 500, 700],
        cssVariable: "--font-ibm-plex-sans",
        fallbacks: ["sans-serif"]
      },
      {
        provider: fontProviders.fontsource(),
        name: "IBM Plex Mono",
        weights: [400, 500, 700],
        cssVariable: "--font-ibm-plex-mono",
        fallbacks: ["monospace"]
      }
    ]
  }
});
