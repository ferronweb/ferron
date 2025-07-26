// @ts-check
import { defineConfig, fontProviders } from "astro/config";

import tailwindcss from "@tailwindcss/vite";
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
    sitemap(),
    pagefind(),
    (await import("astro-compress")).default({
      HTML: true // This setting wouldn't work with React (it would cause hydration errors), but since the website uses vanilla JS, it's safe to enable.
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
        name: "Spline Sans",
        weights: [400, 500, 600, 700],
        cssVariable: "--font-spline-sans",
        fallbacks: ["sans-serif"],
        subsets: ["latin", "latin-ext"]
      },
      {
        provider: fontProviders.fontsource(),
        name: "Spline Sans Mono",
        weights: [400, 600],
        cssVariable: "--font-spline-sans-mono",
        fallbacks: ["monospace"],
        subsets: ["latin", "latin-ext"]
      }
    ]
  }
});
