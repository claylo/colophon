import type { APIRoute, GetStaticPaths } from "astro";
import { getCollection } from "astro:content";

export const getStaticPaths: GetStaticPaths = async () => {
  const docs = await getCollection("docs");
  return docs.map((doc) => ({
    params: { slug: doc.id === "index" ? undefined : doc.id },
    props: { doc },
  }));
};

export const GET: APIRoute = async ({ props, url }) => {
  const { doc } = props as { doc: Awaited<ReturnType<typeof getCollection>>[number] };

  const canonicalUrl = url.href.replace(/\.md$/, "/");

  const frontmatter = `---
title: "${doc.data.title}"${doc.data.description ? `\ndescription: "${doc.data.description}"` : ""}
url: ${canonicalUrl}
---`;

  const markdown = `${frontmatter}\n\n${doc.body}`;

  return new Response(markdown, {
    headers: {
      "Content-Type": "text/markdown; charset=utf-8",
      "Content-Disposition": `inline; filename="${doc.id}.md"`,
    },
  });
};
