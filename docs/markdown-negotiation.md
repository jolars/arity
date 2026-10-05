# Markdown content negotiation

The documentation workflow builds Markdown variants under `docs/book/_markdown/`
from mdBook's rendered main content. The Cloudflare Worker in
`docs/markdown-worker.mjs` serves those files at the original page URLs when the
request includes `Accept: text/markdown`. It passes HTML through for browser
requests and preserves non-page assets.

GitHub Pages cannot negotiate responses by request header. To activate the
Worker on `arity.cc`:

1. Make the `arity.cc` DNS records proxied in Cloudflare. The records currently
   point directly to GitHub Pages and bypass Cloudflare's request processing.
2. Add `CLOUDFLARE_API_TOKEN` (Workers Scripts edit and Workers Routes edit for
   this zone) and `CLOUDFLARE_ACCOUNT_ID` as repository Actions secrets.
3. Run the Documentation workflow. It deploys the Pages artifact first, then
   deploys the Worker on the `arity.cc/*` route.

Check the public responses after deployment:

```sh
curl -i -H 'Accept: text/markdown' https://arity.cc/
curl -i -H 'Accept: text/markdown' https://arity.cc/guide/editors.html
curl -i https://arity.cc/guide/editors.html
```

The first two responses should have `Content-Type: text/markdown; charset=utf-8`
and `Vary: Accept`; the last should remain HTML. Run the local checks with
`node --test docs/markdown-worker.test.mjs` and
`python3 docs/build_markdown.py docs/book` after building the book.
