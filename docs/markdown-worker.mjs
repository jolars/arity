// Static assets are the origin for both browser and Markdown responses.
const redirects = new Map([
  ["/getting-started.html", "/installation.html"],
  ["/_markdown/getting-started.md", "/_markdown/installation.md"],
  ["/reference/benchmarks.html", "/guide/performance.html"],
  ["/reference/suppression.html", "/reference/directives.html"],
]);
function acceptsMarkdown(header) {
  return (header ?? "").split(",").some((part) => {
    const [type, ...parameters] = part.trim().split(";");
    if (type.toLowerCase() !== "text/markdown") return false;
    const quality = parameters.find((parameter) =>
      /^\s*q\s*=/i.test(parameter),
    );
    return !quality || Number(quality.split("=")[1]) > 0;
  });
}

function markdownPath(pathname) {
  if (pathname === "/") return "/_markdown/index.md";
  if (pathname.endsWith("/")) return `/_markdown${pathname}index.md`;
  if (pathname.endsWith(".html"))
    return `/_markdown${pathname.slice(0, -5)}.md`;
  return null;
}

function withVary(headers) {
  const vary = headers.get("vary");
  if (
    !vary?.split(",").some((name) => name.trim().toLowerCase() === "accept")
  ) {
    headers.set("vary", vary ? `${vary}, Accept` : "Accept");
  }
  return headers;
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const target = redirects.get(url.pathname);
    if (target) {
      url.pathname = target;
      return Response.redirect(url.href, 301);
    }
    const fetchOrigin = (assetRequest) => {
      const assetUrl = new URL(assetRequest.url);
      // With HTML normalization disabled, directory indexes are explicit.
      if (assetUrl.pathname === "/") assetUrl.pathname = "/index.html";
      return env.ASSETS.fetch(new Request(assetUrl, assetRequest));
    };
    const page = markdownPath(new URL(request.url).pathname);
    if (!page || !["GET", "HEAD"].includes(request.method))
      return fetchOrigin(request);

    const htmlHeaders = new Headers(request.headers);
    htmlHeaders.set("accept", "text/html");
    if (acceptsMarkdown(request.headers.get("accept"))) {
      // HTML validators and byte ranges describe a different representation.
      for (const name of [
        "if-none-match",
        "if-modified-since",
        "if-range",
        "range",
      ]) {
        htmlHeaders.delete(name);
      }
    }
    const html = await fetchOrigin(
      new Request(request, { method: "GET", headers: htmlHeaders }),
    );
    const htmlResponse = () =>
      new Response(request.method === "HEAD" ? null : html.body, {
        status: html.status,
        statusText: html.statusText,
        headers: withVary(new Headers(html.headers)),
      });
    if (
      !acceptsMarkdown(request.headers.get("accept")) ||
      !html.ok ||
      !html.headers.get("content-type")?.toLowerCase().startsWith("text/html")
    ) {
      return htmlResponse();
    }

    const sourceUrl = new URL(page, request.url);
    const source = await fetchOrigin(new Request(sourceUrl, { method: "GET" }));
    if (!source.ok) return htmlResponse();

    const headers = withVary(new Headers(html.headers));
    for (const name of [
      "content-encoding",
      "content-length",
      "content-range",
      "etag",
      "last-modified",
      "transfer-encoding",
      "accept-ranges",
    ])
      headers.delete(name);
    headers.set("content-type", "text/markdown; charset=utf-8");
    await html.body?.cancel();
    return new Response(request.method === "HEAD" ? null : source.body, {
      status: html.status,
      statusText: html.statusText,
      headers,
    });
  },
};
