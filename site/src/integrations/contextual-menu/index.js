import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { readFileSync } from "node:fs";

const __dirname = dirname(fileURLToPath(import.meta.url));

const normalizeConfig = (options = {}) => ({
  actions: ["copy", "view"],
  ...options,
});

function starlightContextualMenuIntegration(config) {
  const normalizedConfig = normalizeConfig(config);

  return {
    name: "contextual-menu",
    hooks: {
      "astro:config:setup": async ({ injectScript }) => {
        const contextualMenuContent = readFileSync(
          join(__dirname, "contextual-menu.js"),
          "utf-8"
        );

        injectScript(
          "page",
          `
            ${contextualMenuContent};
            initContextualMenu(${JSON.stringify({
              actions: normalizedConfig.actions,
              hideMainActionLabel: config.hideMainActionLabel,
            })});
          `
        );
      },
    },
  };
}

export default function starlightContextualMenu(userConfig) {
  const config = normalizeConfig(userConfig);

  return {
    name: "contextual-menu-plugin",
    hooks: {
      "config:setup"({ addIntegration }) {
        addIntegration(starlightContextualMenuIntegration(config));
      },
    },
  };
}
