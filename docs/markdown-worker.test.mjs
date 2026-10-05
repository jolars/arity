import assert from 'node:assert/strict';
import test from 'node:test';
import worker from './markdown-worker.mjs';

const seen = [];
const origin = async (request) => {
  const url = new URL(request.url);
  seen.push(url.pathname);
  if (url.pathname === '/_markdown/index.md') {
    return new Response('# Arity\n\nIntroduction.\n', { headers: { 'content-type': 'text/plain' } });
  }
  if (url.pathname === '/_markdown/guide/editors.md') {
    return new Response('# Editor Setup\n', { headers: { 'content-type': 'text/plain' } });
  }
  if (url.pathname === '/missing.html') {
    return new Response('Page not found', { status: 404, headers: { 'content-type': 'text/html' } });
  }
  if (url.pathname.startsWith('/_markdown/')) {
    return new Response('No variant', { status: 404 });
  }
  return new Response('<html><main>Browser page</main></html>', {
    headers: { 'content-type': 'text/html; charset=utf-8', vary: 'Accept-Encoding', etag: 'html-tag' },
  });
};

test('serves markdown at the original page URL and keeps cache variants distinct', async () => {
  seen.length = 0;
  const response = await worker.fetch(new Request('https://arity.cc/guide/editors.html', {
    headers: { accept: 'text/html, text/markdown;q=0.8' },
  }), {}, { fetch: origin });
  assert.equal(response.headers.get('content-type'), 'text/markdown; charset=utf-8');
  assert.equal(response.headers.get('vary'), 'Accept-Encoding, Accept');
  assert.equal(response.headers.get('etag'), null);
  assert.equal(await response.text(), '# Editor Setup\n');
  assert.deepEqual(seen, ['/guide/editors.html', '/_markdown/guide/editors.md']);
});

test('serves HTML by default and when markdown has zero quality', async () => {
  for (const accept of [null, 'text/markdown;q=0, text/html']) {
    const headers = accept ? { accept } : {};
    const response = await worker.fetch(new Request('https://arity.cc/', { headers }), {}, { fetch: origin });
    assert.match(response.headers.get('content-type'), /^text\/html/);
    assert.equal(response.headers.get('vary'), 'Accept-Encoding, Accept');
    assert.match(await response.text(), /Browser page/);
  }
});

test('uses the introduction for the root URL and leaves other assets alone', async () => {
  seen.length = 0;
  const response = await worker.fetch(new Request('https://arity.cc/', {
    headers: { accept: 'text/markdown' },
  }), {}, { fetch: origin });
  assert.match(await response.text(), /^# Arity/);
  assert.deepEqual(seen, ['/', '/_markdown/index.md']);

  seen.length = 0;
  const asset = await worker.fetch(new Request('https://arity.cc/arity.schema.json', {
    headers: { accept: 'text/markdown' },
  }), {}, { fetch: origin });
  assert.match(await asset.text(), /Browser page/);
  assert.deepEqual(seen, ['/arity.schema.json']);
});

test('keeps origin errors and missing variants intact', async () => {
  const request = (path) => new Request(`https://arity.cc/${path}`, {
    headers: { accept: 'text/markdown' },
  });
  const missingPage = await worker.fetch(request('missing.html'), {}, { fetch: origin });
  assert.equal(missingPage.status, 404);
  assert.equal(await missingPage.text(), 'Page not found');

  const missingVariant = await worker.fetch(request('guide/other.html'), {}, { fetch: origin });
  assert.equal(missingVariant.headers.get('content-type'), 'text/html; charset=utf-8');
  assert.match(await missingVariant.text(), /Browser page/);
});
