import { visit } from "unist-util-visit";

const alertRegex = /^\[!(?<type>\w+)]\s*/;

/**
 * GitHub alert type → Starlight aside variant mapping.
 *
 * GitHub defines five alert types. Starlight has four aside variants:
 * note, tip, caution, danger. We map GitHub's types to the closest
 * Starlight equivalents.
 */
const DEFAULT_TYPES = {
  note: "note",
  tip: "tip",
  important: "caution",
  warning: "caution",
  caution: "danger",
};

/** Human-readable labels for each Starlight aside variant. */
const LABELS = {
  note: "Note",
  tip: "Tip",
  caution: "Caution",
  danger: "Danger",
};

/**
 * Parse a Starlight icon SVG string (e.g. '<path d="..."/>') into HAST
 * children for an <svg> element. Returns the full <svg> HAST node, or
 * null if no icon string is provided.
 */
function makeIconNode(svgInner) {
  if (!svgInner) return null;

  // Starlight icon strings contain one or more <path .../> elements.
  // Parse them into HAST nodes with a simple regex — these are trusted
  // strings from Starlight's own source, not user input.
  const pathRegex = /<path\s+([^>]*?)\/?\s*>/g;
  const attrRegex = /([\w-]+)="([^"]*)"/g;
  const children = [];
  let pathMatch;

  while ((pathMatch = pathRegex.exec(svgInner)) !== null) {
    const properties = {};
    let attrMatch;
    while ((attrMatch = attrRegex.exec(pathMatch[1])) !== null) {
      properties[attrMatch[1]] = attrMatch[2];
    }
    children.push({
      type: "element",
      tagName: "path",
      properties,
      children: [],
    });
  }

  if (children.length === 0) return null;

  return {
    type: "element",
    tagName: "svg",
    properties: {
      viewBox: "0 0 24 24",
      width: "16",
      height: "16",
      fill: "currentColor",
      className: ["starlight-aside__icon"],
    },
    children,
  };
}

/**
 * Rehype plugin that transforms GitHub-style alert blockquotes into
 * Starlight-styled aside elements.
 *
 * Supports the standard GitHub alert syntax inside blockquotes:
 *
 *   > [!NOTE]
 *   > This is a note.
 *
 *   > [!TIP]
 *   > This is a tip.
 *
 * ## Why this plugin exists
 *
 * Starlight has built-in aside support via `:::note` directive syntax
 * (remarkAsides), but no support for GitHub's `> [!NOTE]` blockquote
 * alert syntax. The docs content uses GitHub-flavored alerts so it
 * renders correctly both here and on GitHub.
 *
 * The npm package `starlight-github-alerts` can't be used because it
 * has its own hardcoded path check against `src/content/docs/`,
 * independent of Starlight's `processedDirs` config. Content loaded
 * from other paths via glob loader is silently skipped.
 *
 * ## Why rehype, not remark?
 *
 * A rehype plugin operates on the HTML AST after markdown conversion.
 * By this point, blockquotes are already `<blockquote>` elements that
 * we can pattern-match against. This avoids two Astro pipeline
 * limitations that block remark-level alternatives:
 *
 * 1. `data.hName`/`data.hProperties` on mdast nodes — Astro ignores.
 * 2. Raw `{ type: "html" }` nodes — Astro strips/sanitizes.
 *
 * The plugin produces the same `<aside class="starlight-aside ...">` HTML
 * structure as Starlight's remarkAsides, including SVG icons loaded from
 * Starlight's built-in icon set. Starlight's existing CSS handles styling.
 *
 * @param {object} [options]
 * @param {Record<string, string>} [options.types] - Override the default
 *   GitHub alert type → Starlight aside variant mapping.
 * @param {Record<string, string>|null} [options.icons] - SVG path strings
 *   per aside variant, loaded from Starlight's BuiltInIcons at build time.
 *   When null, asides render without icons (text-only titles).
 */
export function rehypeGitHubAlerts({ types = DEFAULT_TYPES, icons = null } = {}) {
  return (tree) => {
    visit(tree, "element", (node, index, parent) => {
      if (!parent || index === undefined) return;
      if (node.tagName !== "blockquote") return;

      // The alert marker lives in the first <p>'s first text node.
      const firstP = node.children.find(
        (c) => c.type === "element" && c.tagName === "p"
      );
      if (!firstP) return;

      const firstText = firstP.children.find((c) => c.type === "text");
      if (!firstText) return;

      const match = alertRegex.exec(firstText.value);
      if (!match) return;

      const type = match.groups.type.toLowerCase();
      if (!(type in types)) return;

      const variant = types[type];
      const label = LABELS[variant] || variant;

      // Strip the [!TYPE] prefix from the text content.
      firstText.value = firstText.value.slice(match[0].length);

      // Build the title children: optional SVG icon + label text.
      const titleChildren = [];
      const iconNode = icons && makeIconNode(icons[variant]);
      if (iconNode) titleChildren.push(iconNode);
      titleChildren.push({ type: "text", value: label });

      // Replace the blockquote with a Starlight aside.
      // This produces the same HTML structure as Starlight's remarkAsides:
      //   <aside class="starlight-aside starlight-aside--note" aria-label="Note">
      //     <p class="starlight-aside__title" aria-hidden="true">
      //       <svg ...>...</svg>
      //       Note
      //     </p>
      //     <div class="starlight-aside__content">...original content...</div>
      //   </aside>
      parent.children[index] = {
        type: "element",
        tagName: "aside",
        properties: {
          className: [`starlight-aside`, `starlight-aside--${variant}`],
          ariaLabel: label,
        },
        children: [
          {
            type: "element",
            tagName: "p",
            properties: {
              className: ["starlight-aside__title"],
              ariaHidden: "true",
            },
            children: titleChildren,
          },
          {
            type: "element",
            tagName: "div",
            properties: { className: ["starlight-aside__content"] },
            children: node.children,
          },
        ],
      };
    });
  };
}
