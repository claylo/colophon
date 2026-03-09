import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { rehypeGitHubAlerts } from "./rehype.js";

/**
 * Aside variant → Starlight built-in icon name.
 * Mapping from @astrojs/starlight/user-components/Aside.astro.
 */
const VARIANT_ICONS = {
  note: "information",
  tip: "rocket",
  caution: "warning",
  danger: "error",
};

/**
 * Load icon SVG strings from Starlight's Icons.ts source at build time.
 * Reads the file with Node fs (no Vite module runner dependency) and
 * extracts the 4 icon strings via regex. The SVG values in Icons.ts are
 * single-quoted strings containing <path .../> elements.
 * Returns { note: '<path .../>',  tip: '<path .../>', ... } or null.
 */
function loadStarlightIcons() {
  try {
    const iconsPath = fileURLToPath(
      new URL(
        "../../../node_modules/@astrojs/starlight/components/Icons.ts",
        import.meta.url,
      ),
    );
    const source = readFileSync(iconsPath, "utf8");
    const icons = {};
    for (const [variant, iconName] of Object.entries(VARIANT_ICONS)) {
      const re = new RegExp(`\\b${iconName}:\\s*\\n?\\s*'([^']+)'`);
      const match = source.match(re);
      if (match) icons[variant] = match[1];
    }
    return Object.keys(icons).length > 0 ? icons : null;
  } catch (e) {
    console.warn("[github-alerts] Could not load Starlight icons:", e.message);
    return null;
  }
}

/**
 * Starlight plugin that adds GitHub-style alert syntax (`> [!NOTE]`, etc.)
 * to markdown content, producing the same styled aside components that
 * Starlight's native `:::note` directive syntax generates — including
 * matching SVG icons.
 *
 * This exists because Starlight has no built-in support for GitHub's
 * blockquote alert syntax, and the `starlight-github-alerts` npm package
 * has a hardcoded path check that skips glob-loaded content. See rehype.js
 * for details.
 *
 * ## Usage
 *
 *   import githubAlerts from "./src/integrations/github-alerts/index.js";
 *
 *   starlight({
 *     plugins: [githubAlerts()],
 *   })
 *
 * ## Custom type mapping
 *
 *   githubAlerts({
 *     types: {
 *       note: "note",
 *       tip: "tip",
 *       important: "danger",  // override: render [!IMPORTANT] as danger
 *       warning: "caution",
 *       caution: "danger",
 *     }
 *   })
 *
 * @param {object} [userConfig]
 * @param {Record<string, string>} [userConfig.types] - Map of GitHub alert
 *   types (note, tip, important, warning, caution) to Starlight aside
 *   variants (note, tip, caution, danger).
 */
export default function githubAlerts(userConfig = {}) {
  return {
    name: "github-alerts-plugin",
    hooks: {
      "config:setup"({ addIntegration }) {
        const icons = loadStarlightIcons();

        addIntegration({
          name: "github-alerts",
          hooks: {
            "astro:config:setup": ({ command, config }) => {
              if (command !== "dev" && command !== "build") return;

              // Append as a rehype plugin. Rehype runs after the
              // markdown→HTML conversion, so blockquotes are already
              // <blockquote> elements we can pattern-match against.
              config.markdown.rehypePlugins.push([
                rehypeGitHubAlerts,
                { types: userConfig.types, icons },
              ]);
            },
          },
        });
      },
    },
  };
}
