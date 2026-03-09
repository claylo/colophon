// @ts-check
import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import starlightContextualMenu from "./src/integrations/contextual-menu/index.js";
import githubAlerts from "./src/integrations/github-alerts/index.js";
import rehypeGithubEmoji from 'rehype-github-emoji';
import rehypeBasePath from "./src/plugins/rehype-base-path.js";
import { beasties } from "vite-plugin-beasties";

// https://astro.build/config
export default defineConfig({

  site: "https://claylo.github.io",
  base: "/colophon",

  prefetch: {
    prefetchAll: true,
    defaultStrategy: 'viewport'
  },
  experimental: {
    clientPrerender: true,
  },
  markdown: {
    rehypePlugins: [rehypeGithubEmoji, [rehypeBasePath, "/colophon"]],
  },
  vite: {
    plugins: [
      beasties({
        options: {
          preload: 'media', // no JS used at runtime, pure CSS media queries
          pruneSource: true, // strip inlined rules from lazy-loaded sheet
          mergeStylesheets: true,
          fonts: true, // inline critical @font-face + preload
          keyframes: 'critical',
        },
      }),
    ],
  },
  integrations: [
    starlight({
      title: "colophon",
      social: [
        {
          icon: "document",
          label: "LLMs.txt",
          href: "/colophon/llms.txt"
        },
        {
          icon: "github",
          label: "GitHub",
          href: "https://github.com/claylo/colophon",
        },
      ],
      plugins: [
        starlightContextualMenu({ actions: ["copy", "view", "chatgpt", "claude"] }),
        githubAlerts(),
      ],
      sidebar: [
        {
          label: "Guides",
          autogenerate: { directory: "guides" },
        },
        {
          label: "Reference",
          autogenerate: { directory: "reference" },
        },
      ],
    }),
  ],
});
