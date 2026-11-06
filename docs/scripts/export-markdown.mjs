import { mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { createMarkdownRenderer } from "vitepress";
import { faviconHead } from "./favicon.mjs";
import {
  chapters,
  chartPayloads,
  distRoot,
  expandIncludes,
  pageUrl,
  sourceRoot,
  withoutPlayground,
} from "./content.mjs";

const renderer = await createMarkdownRenderer(sourceRoot);
for (const chapter of chapters()) {
  const file = join(sourceRoot, chapter);
  const source = withoutPlayground(
    expandIncludes(readFileSync(file, "utf8"), file),
  );
  const env = {};
  renderer.render(source, env);
  const frontmatter = env.frontmatter ?? {};
  let markdown = source.replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n/, "");
  if (frontmatter.hero) {
    const { name, text, tagline, actions = [] } = frontmatter.hero;
    markdown =
      [`# ${name}`, text, tagline].filter(Boolean).join("\n\n") +
      "\n\n" +
      actions.map(({ text, link }) => `- [${text}](${link})`).join("\n") +
      "\n\n" +
      (frontmatter.features ?? [])
        .map(({ title, details }) => `## ${title}\n\n${details}`)
        .join("\n\n") +
      "\n\n" +
      markdown;
  }
  markdown = markdown
    .replace(/^::: code-group\s*\r?\n([\s\S]*?)^:::\s*$/gm, (_, group) =>
      group.replace(
        /^```(\S+) \[([^\]]+)\]\s*$/gm,
        (_, language, label) => `### ${label}\n\n\`\`\`${language}`,
      ),
    )
    .replace(
      /<script type="application\/json" class="bench-data">[\s\S]*?<\/script>/g,
      "",
    )
    .replace(/<span id="[^"]+"[^>]*><\/span>/g, "")
    .replace(/\]\(([^\s)]+)\.md(?=[)#])/g, "]($1.html");
  const target = join(distRoot, "_markdown", chapter);
  mkdirSync(dirname(target), { recursive: true });
  writeFileSync(target, markdown.trim() + "\n");
}

// Preserve mdBook's public print view and contents URL without indexing copies.
const printable = [];
const contents = [];
for (const chapter of chapters().filter((chapter) => chapter !== "index.md")) {
  const file = join(sourceRoot, chapter);
  const source = withoutPlayground(
    expandIncludes(readFileSync(file, "utf8"), file),
  );
  const title = source.match(/^# (.+)$/m)?.[1].replaceAll("`", "") ?? chapter;
  contents.push(
    `<li><a href="${pageUrl(chapter)}">${renderer.utils.escapeHtml(title)}</a></li>`,
  );
  const absoluteLinks = source.replace(
    /\]\(([^\s)]+\.md)(#[^)]*)?\)/g,
    (_, target, hash = "") => {
      if (/^https?:/.test(target)) return `](${target}${hash})`;
      const path = relative(
        sourceRoot,
        resolve(dirname(file), target),
      ).replaceAll("\\", "/");
      return `](${pageUrl(path)}${hash})`;
    },
  );
  printable.push(
    renderer
      .render(chartPayloads(absoluteLinks, renderer.utils.escapeHtml))
      .replaceAll("<details", "<details open"),
  );
}
const stylesheet = readdirSync(join(distRoot, "assets")).find((file) =>
  /^style\..*\.css$/.test(file),
);
const faviconHtml = faviconHead
  .map(
    ([tag, attributes]) =>
      `<${tag} ${Object.entries(attributes)
        .map(([name, value]) => `${name}="${renderer.utils.escapeHtml(value)}"`)
        .join(" ")}>`,
  )
  .join("");
for (const [name, body] of [
  ["print", printable.join("\n<hr>\n")],
  ["toc", `<h1>Contents</h1><ul>${contents.join("\n")}</ul>`],
]) {
  writeFileSync(
    join(distRoot, `${name}.html`),
    `<!doctype html><html lang="en-US"><head><meta charset="utf-8">${faviconHtml}<meta name="viewport" content="width=device-width, initial-scale=1"><meta name="robots" content="noindex"><title>Arity ${name === "print" ? "documentation" : "contents"}</title><link rel="stylesheet" href="/assets/${stylesheet}"><style>main{max-width:900px;margin:auto;padding:24px}.bench-data[hidden]{display:none}.bench-chart{display:none}@media print{main{padding:0}}</style></head><body><main class="vp-doc">${body}</main></body></html>`,
  );
}
