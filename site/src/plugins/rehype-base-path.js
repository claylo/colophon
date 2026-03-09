/**
 * Rehype plugin that prepends the site's base path to absolute internal links.
 * Fixes the classic static-site base path problem in one spot.
 */
import { visit } from "unist-util-visit";

export default function rehypeBasePath(base = "/") {
  const prefix = base.replace(/\/$/, "");
  if (!prefix) return () => {};

  return (tree) => {
    visit(tree, "element", (node) => {
      if (node.tagName === "a" && typeof node.properties?.href === "string") {
        const href = node.properties.href;
        if (href.startsWith("/") && !href.startsWith("//") && !href.startsWith(prefix)) {
          node.properties.href = prefix + href;
        }
      }
    });
  };
}
