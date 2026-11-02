import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { defineConfig } from "vitepress";
import { legacyAliases } from "../scripts/anchors.mjs";
import {
  chartPayloads,
  description,
  origin,
  pageUrl,
  partials,
  sourceRoot,
} from "../scripts/content.mjs";

const repository = "https://github.com/jolars/arity";
const version = readFileSync(join(sourceRoot, "version.md"), "utf8").trim();
const guide = [
  "Formatting",
  "Linting",
  "Configuration",
  "Editor setup",
  "Integrations",
  "Performance",
];
const guidePaths = [
  "formatting",
  "linting",
  "configuration",
  "editors",
  "integrations",
  "performance",
];
const reference = [
  "CLI reference",
  "Configuration reference",
  "Lint rules",
  "Code actions",
  "Directives",
];
const referencePaths = [
  "cli",
  "configuration",
  "rules",
  "code-actions",
  "directives",
];

export default defineConfig({
  title: "Arity",
  description,
  lang: "en-US",
  srcDir: "src",
  srcExclude: partials,
  cleanUrls: false,
  lastUpdated: true,
  vite: { publicDir: join(sourceRoot, "../public") },
  markdown: {
    config(md) {
      const render = md.renderer.rules.heading_open;
      md.renderer.rules.heading_open = (
        tokens,
        index,
        options,
        env,
        renderer,
      ) => {
        const alias = legacyAliases.get(tokens[index].attrGet("id"));
        const heading = render
          ? render(tokens, index, options, env, renderer)
          : renderer.renderToken(tokens, index, options);
        return (
          heading +
          (alias
            ? `<span id="${md.utils.escapeHtml(alias)}" aria-hidden="true"></span>`
            : "")
        );
      };
      md.core.ruler.before("normalize", "arity-charts", (state) => {
        state.src = chartPayloads(state.src, md.utils.escapeHtml);
      });
    },
  },
  head: [
    ["link", { rel: "icon", href: "/favicon.svg", type: "image/svg+xml" }],
    ["meta", { property: "og:type", content: "website" }],
    ["meta", { property: "og:site_name", content: "Arity" }],
    ["meta", { property: "og:image", content: `${origin}/images/og.png` }],
    ["meta", { property: "og:image:type", content: "image/png" }],
    ["meta", { property: "og:image:width", content: "1200" }],
    ["meta", { property: "og:image:height", content: "630" }],
    [
      "meta",
      {
        property: "og:image:alt",
        content: "Arity's code logo beside the name Arity.",
      },
    ],
    ["meta", { name: "twitter:card", content: "summary_large_image" }],
    ["meta", { name: "twitter:image", content: `${origin}/images/og.png` }],
  ],
  transformPageData(page) {
    if (page.relativePath === "404.md") return;
    const title = page.title === "Arity" ? "Arity" : `${page.title} | Arity`;
    const url = origin + pageUrl(page.relativePath);
    const summary = page.description || description;
    page.frontmatter.head ??= [];
    page.frontmatter.head.push(
      ["link", { rel: "canonical", href: url }],
      ["meta", { property: "og:url", content: url }],
      ["meta", { property: "og:title", content: title }],
      ["meta", { property: "og:description", content: summary }],
      ["meta", { name: "twitter:title", content: title }],
      ["meta", { name: "twitter:description", content: summary }],
    );
  },
  sitemap: {
    hostname: origin,
    transformItems(items) {
      return items.map((item) => {
        const source =
          item.url === "" ? "index.md" : item.url.replace(/\.html$/, ".md");
        try {
          const date = execFileSync(
            "git",
            ["log", "-1", "--format=%cs", "--", join(sourceRoot, source)],
            { encoding: "utf8" },
          ).trim();
          if (date) item.lastmod = date;
        } catch {
          /* Source archives have no Git history. */
        }
        return item;
      });
    },
  },
  themeConfig: {
    logo: { src: "/images/logo.png", alt: "" },
    nav: [
      { text: "Guide", link: "/introduction.html" },
      { text: "Reference", link: "/reference/cli.html" },
      { text: "Playground", link: "/playground.html" },
      { text: "Performance", link: "/guide/performance.html" },
    ],
    sidebar: [
      {
        text: "Introduction",
        items: [
          { text: "Overview", link: "/introduction.html" },
          { text: "Getting started", link: "/guide/getting-started.html" },
          { text: "Installation", link: "/installation.html" },
          { text: "Playground", link: "/playground.html" },
        ],
      },
      {
        text: "Guide",
        items: guide.map((text, index) => ({
          text,
          link: `/guide/${guidePaths[index]}.html`,
        })),
      },
      {
        text: "Reference",
        items: reference.map((text, index) => ({
          text,
          link: `/reference/${referencePaths[index]}.html`,
        })),
      },
    ],
    outline: [2, 3],
    socialLinks: [{ icon: "github", link: repository }],
    editLink: { pattern: `${repository}/edit/main/docs/src/:path` },
    search: { provider: "local" },
    footer: { message: `${version} · Released under the MIT license.` },
  },
});
