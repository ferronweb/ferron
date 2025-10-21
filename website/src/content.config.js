import { defineCollection, z } from "astro:content";
import { glob } from "astro/loaders"; // Not available with legacy API

const docs = defineCollection({
  loader: glob({ pattern: "**/!(README).md", base: "../docs" }),
  schema: z.object({
    title: z.string().optional()
  })
});

export const collections = { docs };
