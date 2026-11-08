import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
  chapters,
  distRoot,
  docsRoot,
  origin,
  pageUrl,
  projectRoot,
} from "./content.mjs";

const legacy = JSON.parse(
  readFileSync(join(docsRoot, "tests/legacy-routes.json"), "utf8"),
);
const pages = new Map(
  chapters().map((chapter) => [
    pageUrl(chapter),
    readFileSync(join(distRoot, chapter.replace(/\.md$/, ".html")), "utf8"),
  ]),
);
pages.set("/index.html", pages.get("/"));
const contributing = pages.get("/development/contributing.html");
assert.ok(contributing, "Missing contributing guide");
assert.match(
  contributing,
  /href="https:\/\/github\.com\/jolars\/arity\/blob\/main\/AGENTS\.md"/,
);
assert.match(contributing, /href="\/development\/changelog\.html"/);
assert.match(
  contributing,
  /href="https:\/\/github\.com\/jolars\/arity\/edit\/main\/CONTRIBUTING\.md"/,
);
assert.ok(pages.has("/development/changelog.html"), "Missing changelog");
const changelog = readFileSync(
  join(distRoot, "_markdown/development/changelog.md"),
  "utf8",
);
assert.match(changelog, /^# Changelog\n/);
for (const release of readFileSync(
  join(projectRoot, "CHANGELOG.md"),
  "utf8",
).matchAll(/^## .+$/gm))
  assert.ok(changelog.includes(release[0]), `Missing release: ${release[0]}`);
assert.match(
  readFileSync(join(distRoot, "_markdown/development/contributing.md"), "utf8"),
  /\]\(https:\/\/github\.com\/jolars\/arity\/blob\/main\/AGENTS\.md\)/,
);
const ids = (html) =>
  new Set([...html.matchAll(/\bid="([^"]+)"/g)].map((match) => match[1]));
const failures = [];
for (const [path, anchors] of Object.entries(legacy.pages)) {
  const html = pages.get(legacy.redirects[path] ?? path);
  if (!html) {
    failures.push(`Missing legacy page: ${path}`);
    continue;
  }
  for (const anchor of anchors) {
    if (!ids(html).has(anchor))
      failures.push(`Missing legacy anchor: ${path}#${anchor}`);
  }
}
for (const [path, html] of pages) {
  const url = origin + (path === "/index.html" ? "/" : path);
  if (!html.includes(`rel="canonical" href="${url}"`))
    failures.push(`Missing canonical: ${path}`);
  if (!html.includes(`property="og:url" content="${url}"`))
    failures.push(`Missing Open Graph URL: ${path}`);
  for (const match of html.matchAll(/<a\b[^>]*\bhref="([^"]+)"/g)) {
    const target = new URL(match[1].replaceAll("&amp;", "&"), origin + path);
    if (target.origin !== origin) continue;
    const destination = legacy.redirects[target.pathname] ?? target.pathname;
    if (!pages.has(destination)) {
      if (destination.endsWith(".html") || destination.endsWith("/"))
        failures.push(`Broken link: ${path} -> ${target.pathname}`);
      continue;
    }
    if (
      target.hash &&
      !ids(pages.get(destination)).has(decodeURIComponent(target.hash.slice(1)))
    )
      failures.push(
        `Broken anchor: ${path} -> ${target.pathname}${target.hash}`,
      );
  }
}
for (const [source, target] of [
  ["scripts/arity-installer.sh", "install"],
  ["scripts/arity-installer.ps1", "install.ps1"],
  ["arity.schema.json", "arity.schema.json"],
])
  assert.deepEqual(
    readFileSync(join(distRoot, target)),
    readFileSync(join(projectRoot, source)),
    target,
  );
for (const chapter of chapters()) {
  const markdown = readFileSync(join(distRoot, "_markdown", chapter), "utf8");
  assert.ok(markdown.trim(), chapter);
  assert.doesNotMatch(
    markdown,
    /<!--\s*@include|\{\{#include|layout: home/,
    chapter,
  );
}
const home = readFileSync(join(distRoot, "_markdown/index.md"), "utf8");
assert.match(home, /^# Arity\n/);
assert.doesNotMatch(home, /\bundefined\b/);
assert.match(home, /\[What is Arity\?\]\(\/introduction\.html\)/);
assert.match(home, /\[Get started\]\(\/guide\/getting-started\.html\)/);
assert.match(home, /\[Source code\]\(https:\/\/github\.com\/jolars\/arity\)/);
const sitemap = readFileSync(join(distRoot, "sitemap.xml"), "utf8");
for (const path of pages.keys())
  if (path !== "/index.html")
    assert.ok(
      sitemap.includes(origin + path),
      `Missing sitemap route: ${path}`,
    );
assert.doesNotMatch(
  sitemap,
  /benchmarks_results|benchmarks_meta|benchmarks_lsp|version\.html|404\.html/,
);
assert.ok(readFileSync(join(distRoot, "404.html"), "utf8").includes("404"));
assert.equal(failures.length, 0, failures.join("\n"));
console.log(
  `Checked ${pages.size - 1} pages, all legacy headings, internal links, Markdown exports, metadata, and downloads.`,
);
