import type { APIRoute } from "astro";
import { getCollection } from "astro:content";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const siteRoot = join(__dirname, "..", "..");
const repoRoot = join(siteRoot, "..");

const pkg = JSON.parse(readFileSync(join(siteRoot, "package.json"), "utf-8"));
const repoUrl = (pkg.repository?.url ?? pkg.repository ?? "")
  .replace(/^git\+/, "")
  .replace(/\.git$/, "");
const GITHUB_RAW = repoUrl.replace("github.com", "raw.githubusercontent.com") + "/main";

type Doc = { id: string; data: { title?: string; description?: string }; body?: string };

function baseUrl(site: URL | undefined): string {
  const origin = site?.href.replace(/\/$/, "") ?? "";
  const base = import.meta.env.BASE_URL.replace(/\/$/, "");
  return `${origin}${base}`;
}

function resolveScreenshots(content: string): string {
  return content.replace(
    /!\[([^\]]*)\]\((docs\/assets\/[^)]+)\)/g,
    (_, alt, path) => `![${alt}](${GITHUB_RAW}/${path})`
  );
}

/** Group docs by first path segment, ordered by llms.sections from package.json. Full body included. */
function buildSections(docs: Doc[], siteBase: string): string[] {
  const groups = new Map<string, { heading: string; entries: Doc[] }>();

  for (const doc of docs) {
    if (doc.id === "index" || doc.id === "README") continue;
    const sep = doc.id.indexOf("/");
    if (sep === -1) continue;

    const prefix = doc.id.slice(0, sep);
    if (!groups.has(prefix)) {
      groups.set(prefix, { heading: prefix.charAt(0).toUpperCase() + prefix.slice(1), entries: [] });
    }
    const group = groups.get(prefix)!;

    if (doc.id === `${prefix}/README`) {
      group.heading = doc.data.title ?? group.heading;
    } else {
      group.entries.push(doc);
    }
  }

  const order: string[] = pkg.llms?.sections ?? [];
  const sorted = [
    ...order.filter((s: string) => groups.has(s)),
    ...[...groups.keys()].filter((s) => !order.includes(s)),
  ];

  const lines: string[] = [];
  for (const key of sorted) {
    const { heading, entries } = groups.get(key)!;
    lines.push("---", "", `## ${heading}`, "");
    for (const doc of entries.sort((a, b) => (a.data.title ?? a.id).localeCompare(b.data.title ?? b.id))) {
      const url = `${siteBase}/${doc.id}/`;
      const title = doc.data.title ?? doc.id;
      const desc = doc.data.description ? `\n> ${doc.data.description}` : "";
      const body = doc.body?.trim() ?? "";
      lines.push(`### [${title}](${url})${desc}`, "", body, "");
    }
  }

  return lines;
}

export const GET: APIRoute = async ({ site }) => {
  const docs = await getCollection("docs");
  const siteBase = baseUrl(site);

  const readme = readFileSync(join(repoRoot, "README.md"), "utf-8");
  const resolvedReadme = resolveScreenshots(readme);

  const lines: string[] = [
    `# ${pkg.name} — Full Documentation`,
    "",
    `> ${pkg.llms?.tagline ?? pkg.description}`,
    "",
    `- [Project README](${GITHUB_RAW}/README.md)`,
    `- [Source](${repoUrl})`,
    `- [npm](https://www.npmjs.com/package/${pkg.name})`,
    `- [Documentation](${siteBase}/)`,
    "",
    "---",
    "",
    "## README",
    "",
    resolvedReadme,
    "",
    ...buildSections(docs, siteBase),
  ];

  return new Response(lines.join("\n"), {
    headers: {
      "Content-Type": "text/markdown; charset=utf-8",
    },
  });
};
