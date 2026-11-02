import assert from "node:assert/strict";
import test from "node:test";
import {
  chartPayloads,
  expandIncludes,
  pageUrl,
  withoutPlayground,
} from "../scripts/content.mjs";

test("keeps chart JSON inert and preserves literal dollar replacement sequences", () => {
  const json = '[{"document":"$& <script> \\\"quoted\\\""}]';
  const source = `<script type="application/json" class="bench-data">${json}</script>`;
  const escaped = (s) => s.replaceAll("&", "&amp;").replaceAll("<", "&lt;");
  const actual = chartPayloads(source, escaped);
  assert.equal(
    actual,
    `<pre class="bench-data" hidden v-pre>${escaped(json)}</pre>`,
  );
});

test("keeps published HTML paths and rejects cyclic includes", () => {
  assert.equal(pageUrl("index.md"), "/");
  assert.equal(pageUrl("guide/editors.md"), "/guide/editors.html");
  assert.throws(
    () =>
      expandIncludes(
        "<!--@include: ./content.test.mjs -->",
        import.meta.filename,
      ),
    /Circular/,
  );
});

test("playground exports retain prose without client-only components", () => {
  const source =
    "# Playground\n\nYour code stays in your browser.\n\n<ArityPlayground />\n";
  assert.equal(
    withoutPlayground(source),
    "# Playground\n\nYour code stays in your browser.\n\n",
  );
});
