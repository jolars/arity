import assert from "node:assert/strict";
import test from "node:test";
import worker from "./markdown-worker.mjs";

const seen = [];
const origin = async (request) => {
  const url = new URL(request.url);
  seen.push(url.pathname);
  if (url.pathname === "/_markdown/index.md") {
    return new Response("# Arity\n\nIntroduction.\n", {
      headers: { "content-type": "text/plain" },
    });
  }
  if (url.pathname === "/_markdown/guide/editors.md") {
    return new Response("# Editor setup\n", {
      headers: { "content-type": "text/plain" },
    });
  }
  if (url.pathname === "/missing.html") {
    return new Response("Page not found", {
      status: 404,
      headers: { "content-type": "text/html" },
    });
  }
  if (url.pathname.startsWith("/_markdown/")) {
    return new Response("No variant", { status: 404 });
  }
  return new Response("<html><main>Browser page</main></html>", {
    headers: {
      "content-type": "text/html; charset=utf-8",
      vary: "Accept-Encoding",
      etag: "html-tag",
    },
  });
};

test("serves markdown at the original page URL and keeps cache variants distinct", async () => {
  seen.length = 0;
  const response = await worker.fetch(
    new Request("https://arity.cc/guide/editors.html", {
      headers: { accept: "text/html, text/markdown;q=0.8" },
    }),
    { ASSETS: { fetch: origin } },
  );
  assert.equal(
    response.headers.get("content-type"),
    "text/markdown; charset=utf-8",
  );
  assert.equal(response.headers.get("vary"), "Accept-Encoding, Accept");
  assert.equal(response.headers.get("etag"), null);
  assert.equal(await response.text(), "# Editor setup\n");
  assert.deepEqual(seen, [
    "/guide/editors.html",
    "/_markdown/guide/editors.md",
  ]);
});

test("serves HTML by default and when markdown has zero quality", async () => {
  for (const accept of [null, "text/markdown;q=0, text/html"]) {
    const headers = accept ? { accept } : {};
    const response = await worker.fetch(
      new Request("https://arity.cc/", { headers }),
      { ASSETS: { fetch: origin } },
    );
    assert.match(response.headers.get("content-type"), /^text\/html/);
    assert.equal(response.headers.get("vary"), "Accept-Encoding, Accept");
    assert.match(await response.text(), /Browser page/);
  }
});

test("uses the introduction for the root URL and leaves other assets alone", async () => {
  seen.length = 0;
  const response = await worker.fetch(
    new Request("https://arity.cc/", {
      headers: { accept: "text/markdown" },
    }),
    { ASSETS: { fetch: origin } },
  );
  assert.match(await response.text(), /^# Arity/);
  assert.deepEqual(seen, ["/index.html", "/_markdown/index.md"]);

  seen.length = 0;
  const asset = await worker.fetch(
    new Request("https://arity.cc/arity.schema.json", {
      headers: { accept: "text/markdown" },
    }),
    { ASSETS: { fetch: origin } },
  );
  assert.match(await asset.text(), /Browser page/);
  assert.deepEqual(seen, ["/arity.schema.json"]);
});

test("keeps origin errors and missing variants intact", async () => {
  const request = (path) =>
    new Request(`https://arity.cc/${path}`, {
      headers: { accept: "text/markdown" },
    });
  const missingPage = await worker.fetch(request("missing.html"), {
    ASSETS: { fetch: origin },
  });
  assert.equal(missingPage.status, 404);
  assert.equal(await missingPage.text(), "Page not found");

  const missingVariant = await worker.fetch(request("guide/other.html"), {
    ASSETS: { fetch: origin },
  });
  assert.equal(
    missingVariant.headers.get("content-type"),
    "text/html; charset=utf-8",
  );
  assert.match(await missingVariant.text(), /Browser page/);
});

test("redirects historical paths while retaining queries and explicit fragments", async () => {
  for (const [path, target] of [
    ["/reference/benchmarks.html", "/guide/performance.html"],
    ["/reference/suppression.html", "/reference/directives.html"],
  ]) {
    const response = await worker.fetch(
      new Request(`https://arity.cc${path}?from=old`),
      {
        ASSETS: {
          fetch: () => {
            throw new Error("Redirect must not fetch assets");
          },
        },
      },
    );
    assert.equal(response.status, 301);
    assert.equal(
      response.headers.get("location"),
      `https://arity.cc${target}?from=old`,
    );
  }
});

test("negotiation ignores HTML validators and byte ranges", async () => {
  const response = await worker.fetch(
    new Request("https://arity.cc/guide/editors.html", {
      headers: {
        accept: "text/markdown",
        "if-none-match": "html-tag",
        range: "bytes=0-1",
      },
    }),
    {
      ASSETS: {
        fetch: async (request) => {
          assert.equal(request.headers.get("if-none-match"), null);
          assert.equal(request.headers.get("range"), null);
          return origin(request);
        },
      },
    },
  );
  assert.equal(response.status, 200);
  assert.equal(await response.text(), "# Editor setup\n");
});

test("HEAD negotiates the same type without returning a body", async () => {
  const response = await worker.fetch(
    new Request("https://arity.cc/guide/editors.html", {
      method: "HEAD",
      headers: { accept: "text/markdown" },
    }),
    { ASSETS: { fetch: origin } },
  );
  assert.equal(
    response.headers.get("content-type"),
    "text/markdown; charset=utf-8",
  );
  assert.equal(await response.text(), "");
});

test("keeps published installation links pointing to their moved page", async () => {
  const response = await worker.fetch(
    new Request("https://arity.cc/getting-started.html?from=readme"),
    {
      ASSETS: {
        fetch: async () => new Response("<h1>Old installation page</h1>"),
      },
    },
  );
  assert.equal(response.status, 301);
  assert.equal(
    response.headers.get("location"),
    "https://arity.cc/installation.html?from=readme",
  );
});
