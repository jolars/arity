import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const docsRoot = fileURLToPath(new URL("../", import.meta.url));
export const projectRoot = resolve(docsRoot, "..");
export const sourceRoot = join(docsRoot, "src");
export const distRoot = join(docsRoot, ".vitepress/dist");
export const origin = "https://arity.cc";
export const repository = "https://github.com/jolars/arity";
export const repositoryPages = {
  "development/contributing.md": "CONTRIBUTING.md",
  "development/changelog.md": "CHANGELOG.md",
};
export const description =
  "Arity is a language server, formatter, and linter for R.";
export const partials = [
  "version.md",
  "guide/benchmarks_meta.md",
  "guide/benchmarks_results.md",
  "guide/benchmarks_lsp.md",
];

export function chapters(directory = sourceRoot) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory())
      return entry.name === "public" ? [] : chapters(path);
    const name = relative(sourceRoot, path).replaceAll("\\", "/");
    return name.endsWith(".md") && !partials.includes(name) ? [name] : [];
  });
}

export function pageUrl(path) {
  return "/" + path.replace(/(^|\/)index\.md$/, "$1").replace(/\.md$/, ".html");
}

export function repositoryLinks(source, chapter) {
  if (!repositoryPages[chapter]) return source;
  return source.replace(
    /\]\((?![a-z][a-z\d+.-]*:|[\/#])([^\s)]+)\)/gi,
    (_, target) => {
      const page = Object.entries(repositoryPages).find(
        ([, file]) => file === target,
      )?.[0];
      if (page) return `](${pageUrl(page)})`;
      const kind = statSync(join(projectRoot, target), {
        throwIfNoEntry: false,
      })?.isDirectory()
        ? "tree"
        : "blob";
      return `](${repository}/${kind}/main/${target})`;
    },
  );
}

export function withoutPlayground(source) {
  return source.replace(/^<ArityPlayground\s*\/>\r?\n?/gm, "");
}

export function expandIncludes(source, file, stack = [file]) {
  return source.replace(/<!--\s*@include:\s*(.*?)\s*-->/g, (_, name) => {
    const included = resolve(dirname(file), name);
    if (stack.includes(included))
      throw new Error(`Circular Markdown include: ${included}`);
    return expandIncludes(readFileSync(included, "utf8"), included, [
      ...stack,
      included,
    ]);
  });
}

export function chartPayloads(source, escapeHtml) {
  // Vue drops script tags. Inert, escaped text preserves every JSON byte.
  return source.replace(
    /<script type="application\/json" class="bench-data">([\s\S]*?)<\/script>/g,
    (_, json) =>
      `<pre class="bench-data" hidden v-pre>${escapeHtml(json)}</pre>`,
  );
}
