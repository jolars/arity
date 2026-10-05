// GitHub Pages serves the book; this Worker only selects its Markdown variant.
function acceptsMarkdown(header) {
  return (header ?? '').split(',').some((part) => {
    const [type, ...parameters] = part.trim().split(';');
    if (type.toLowerCase() !== 'text/markdown') return false;
    const quality = parameters.find((parameter) => /^\s*q\s*=/i.test(parameter));
    return !quality || Number(quality.split('=')[1]) > 0;
  });
}

function markdownPath(pathname) {
  if (pathname === '/') return '/_markdown/index.md';
  if (pathname.endsWith('/')) return `/_markdown${pathname}index.md`;
  if (pathname.endsWith('.html')) return `/_markdown${pathname.slice(0, -5)}.md`;
  return null;
}

function withVary(headers) {
  const vary = headers.get('vary');
  if (!vary?.split(',').some((name) => name.trim().toLowerCase() === 'accept')) {
    headers.set('vary', vary ? `${vary}, Accept` : 'Accept');
  }
  return headers;
}

export default {
  async fetch(request, _env, context) {
    const fetchOrigin = context?.fetch ?? globalThis.fetch;
    const page = markdownPath(new URL(request.url).pathname);
    if (!page || !['GET', 'HEAD'].includes(request.method)) return fetchOrigin(request);

    const htmlHeaders = new Headers(request.headers);
    htmlHeaders.set('accept', 'text/html');
    const html = await fetchOrigin(new Request(request, { headers: htmlHeaders }));
    const htmlResponse = () => new Response(html.body, {
      status: html.status,
      statusText: html.statusText,
      headers: withVary(new Headers(html.headers)),
    });
    if (!acceptsMarkdown(request.headers.get('accept')) || !html.ok ||
        !html.headers.get('content-type')?.toLowerCase().startsWith('text/html')) {
      return htmlResponse();
    }

    const sourceUrl = new URL(page, request.url);
    const source = await fetchOrigin(new Request(sourceUrl, { method: 'GET' }));
    if (!source.ok) return htmlResponse();

    const headers = withVary(new Headers(html.headers));
    for (const name of ['content-encoding', 'content-length', 'content-range',
      'etag', 'last-modified', 'transfer-encoding']) headers.delete(name);
    headers.set('content-type', 'text/markdown; charset=utf-8');
    return new Response(request.method === 'HEAD' ? null : source.body, {
      status: html.status,
      statusText: html.statusText,
      headers,
    });
  },
};
