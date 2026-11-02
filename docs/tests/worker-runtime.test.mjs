import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { createServer } from "node:net";
import { join } from "node:path";
import { after, before, test } from "node:test";
import { setTimeout } from "node:timers/promises";
import { docsRoot, projectRoot } from "../scripts/content.mjs";

let server;
let origin;
let logs = "";
before(
  async () => {
    const socket = createServer();
    await new Promise((resolve) => socket.listen(0, "127.0.0.1", resolve));
    const port = socket.address().port;
    await new Promise((resolve) => socket.close(resolve));
    origin = `http://127.0.0.1:${port}`;
    server = spawn(
      process.execPath,
      [
        "node_modules/wrangler/bin/wrangler.js",
        "dev",
        "--local",
        "--ip",
        "127.0.0.1",
        "--port",
        String(port),
        "--inspector-port",
        "0",
        "--show-interactive-dev-session=false",
      ],
      {
        cwd: docsRoot,
        env: {
          ...process.env,
          WRANGLER_SEND_METRICS: "false",
          BROWSER: "none",
        },
        stdio: ["ignore", "pipe", "pipe"],
      },
    );
    for (const stream of [server.stdout, server.stderr])
      stream.on("data", (data) => {
        logs += data;
      });
    for (let attempt = 0; attempt < 100; attempt++) {
      if (server.exitCode !== null) throw new Error(logs);
      try {
        const response = await fetch(origin + "/index.html");
        if (response.ok) {
          await response.body?.cancel();
          return;
        }
      } catch {
        /* Wait for the local runtime to bind its socket. */
      }
      await setTimeout(200);
    }
    throw new Error(`Worker did not start:\n${logs}`);
  },
  { timeout: 30000 },
);
after(async () => {
  if (server && server.exitCode === null) {
    const exited = new Promise((resolve) => server.once("exit", resolve));
    server.kill("SIGTERM");
    await exited;
  }
});

test("the local Worker serves every published page and Markdown variant", async () => {
  const fixture = JSON.parse(
    readFileSync(join(docsRoot, "tests/legacy-routes.json"), "utf8"),
  );
  for (const path of ["/", ...Object.keys(fixture.pages)]) {
    const html = await fetch(origin + path);
    assert.equal(html.status, 200, path);
    assert.match(html.headers.get("content-type"), /text\/html/);
    assert.match(html.headers.get("vary"), /Accept/);
    const text = await html.text();
    for (const id of fixture.pages[path] ?? [])
      assert.ok(text.includes(`id="${id}"`), `${path}#${id}`);
    const markdown = await fetch(origin + path, {
      headers: { accept: "text/markdown" },
    });
    assert.equal(markdown.status, 200, path);
    assert.match(markdown.headers.get("content-type"), /text\/markdown/);
    assert.ok((await markdown.text()).length > 100, path);
  }
});

test("overview, project quickstart, and installation have separate pages", async () => {
  for (const [path, heading] of [
    ["/introduction.html", "# Overview"],
    ["/guide/getting-started.html", "# Getting started"],
    ["/installation.html", "# Installation"],
  ]) {
    const response = await fetch(origin + path, {
      headers: { accept: "text/markdown" },
    });
    assert.equal(response.status, 200, path);
    const markdown = await response.text();
    assert.ok(markdown.startsWith(heading), path);
    if (path === "/guide/getting-started.html") {
      assert.match(markdown, /arity format --check \./);
      assert.doesNotMatch(markdown, /cargo install arity/);
    }
  }
});

test("the playground serves HTML, readable Markdown, and executable Wasm assets", async () => {
  const html = await fetch(`${origin}/playground.html`);
  assert.equal(html.status, 200);
  const page = await html.text();
  assert.match(page, /pg-input/);
  assert.match(page, /pg-output/);
  const markdown = await fetch(`${origin}/playground.html`, {
    headers: { accept: "text/markdown" },
  });
  assert.equal(markdown.status, 200);
  const text = await markdown.text();
  assert.match(text, /^# Playground/);
  assert.match(text, /stays in your browser/);
  assert.doesNotMatch(text, /ArityPlayground|<script/);
  const script = await fetch(`${origin}/playground/pkg/arity_wasm.js`);
  assert.equal(script.status, 200);
  assert.match(script.headers.get("content-type"), /javascript/);
  await script.body?.cancel();
  const wasm = await fetch(`${origin}/playground/pkg/arity_wasm_bg.wasm`);
  assert.equal(wasm.status, 200);
  assert.equal(wasm.headers.get("content-type"), "application/wasm");
  assert.ok(await WebAssembly.compile(await wasm.arrayBuffer()));
  const print = await fetch(`${origin}/print.html`);
  assert.doesNotMatch(await print.text(), /ArityPlayground/);
});

test("the printable documentation and contents retain their public URLs", async () => {
  const print = await fetch(`${origin}/print.html`);
  assert.equal(print.status, 200);
  const html = await print.text();
  assert.match(html, /id="getting-started"/);
  assert.match(html, /id="lint-rules"/);
  assert.match(html, /<details open/);
  const toc = await fetch(`${origin}/toc.html`);
  assert.equal(toc.status, 200);
  assert.match(await toc.text(), /href="\/guide\/editors.html"/);
});

test("legacy redirects preserve queries and destination fragments", async () => {
  for (const [path, target] of Object.entries(
    JSON.parse(readFileSync(join(docsRoot, "tests/legacy-routes.json"), "utf8"))
      .redirects,
  )) {
    const response = await fetch(`${origin}${path}?from=legacy`, {
      redirect: "manual",
    });
    assert.equal(response.status, 301, path);
    assert.equal(
      response.headers.get("location"),
      `${origin}${target}?from=legacy`,
    );
  }
});

test("downloads keep their exact bytes, content types, and schema CORS", async () => {
  for (const [source, target, contentType] of [
    ["scripts/arity-installer.sh", "/install", /text\/plain/],
    ["scripts/arity-installer.ps1", "/install.ps1", /text\/plain/],
    ["arity.schema.json", "/arity.schema.json", /application\/schema\+json/],
  ]) {
    const response = await fetch(origin + target, {
      headers: { accept: "text/markdown" },
    });
    assert.equal(response.status, 200, target);
    assert.match(response.headers.get("content-type"), contentType);
    assert.deepEqual(
      Buffer.from(await response.arrayBuffer()),
      readFileSync(join(projectRoot, source)),
    );
    if (target.endsWith(".json"))
      assert.equal(response.headers.get("access-control-allow-origin"), "*");
  }
  const markdown = await fetch(`${origin}/_markdown/index.md`);
  assert.match(markdown.headers.get("content-type"), /text\/markdown/);
  assert.match(await markdown.text(), /^# Arity/);
});

test("HEAD and conditional Markdown requests return the correct representation", async () => {
  const html = await fetch(`${origin}/guide/editors.html`);
  const etag = html.headers.get("etag");
  await html.body?.cancel();
  for (const method of ["GET", "HEAD"]) {
    const response = await fetch(`${origin}/guide/editors.html`, {
      method,
      headers: {
        accept: "text/markdown",
        "if-none-match": etag ?? "html",
        range: "bytes=0-1",
      },
    });
    assert.equal(response.status, 200);
    assert.match(response.headers.get("content-type"), /text\/markdown/);
    assert.equal(response.headers.get("content-range"), null);
    const body = await response.text();
    if (method === "HEAD") assert.equal(body, "");
    else assert.match(body, /^# Editor setup/);
  }
});

test("unknown pages and assets return real 404 responses", async () => {
  for (const path of [
    "/no-such-page",
    "/no-such-page.html",
    "/missing/",
    "/missing.json",
    "/_markdown/missing.md",
  ]) {
    const response = await fetch(origin + path, {
      headers: { accept: "text/markdown" },
    });
    assert.equal(response.status, 404, path);
    await response.body?.cancel();
  }
});
