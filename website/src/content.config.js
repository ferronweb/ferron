import { defineCollection, z } from "astro:content";
import { glob } from "astro/loaders"; // Not available with legacy API

const blog = defineCollection({
  loader: glob({ pattern: "**/*.md", base: "./src/content/blog" }),
  schema: z.object({
    title: z.string().optional(),
    description: z.string().optional(),
    date: z.date().optional(),
    cover: z.string().optional()
  })
});

const docs = defineCollection({
  loader: glob({ pattern: "**/*.md", base: "./src/content/docs" }),
  schema: z.object({
    title: z.string().optional()
  })
});

export const collections = { blog, docs };
