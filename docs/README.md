# Documentation site

The site uses VitePress and Cloudflare Workers Static Assets. The
`arity-markdown` Worker serves the complete site at `arity.cc`, including HTML,
Markdown variants, installers, and the JSON schema. GitHub Pages is no longer
part of the build or serving path.

## Local development

From the repository root, use:

```sh
compute run -- task docs-preview
compute run -- task docs-check
compute run -- task docs-worker
```

`docs-preview` runs VitePress with live reload. `docs-check` builds the site,
checks published URLs and anchors, checks internal links and metadata, compares
download bytes, tests the local Worker, and runs a deployment dry run.
`docs-worker` previews the built site through Wrangler. Node 22 or later is
required; the development shell provides Node and the NixOS dynamic loader used
by Wrangler's bundled runtime.

The R playground at `/playground.html` uses the unpublished `arity-wasm` crate.
The documentation development and production commands build its JavaScript and
WebAssembly assets automatically with `wasm-pack`, which the development shell
provides. To rebuild just those assets, run
`compute run -- task playground-wasm`. Generated bundles under
`public/playground/pkg/` are ignored. CodeMirror is bundled by Vite and loaded
only when the playground opens. Shared links contain formatting settings, never
the input code.

Navigation lives in `.vitepress/config.mjs`. Generated references and benchmark
partials still come from `task docs-gen`. Includes use VitePress's
`<!--@include: ./file.md -->` syntax. Files copied unchanged live in `public/`;
`npm run build` stages installers and the schema from their authoritative
repository sources. The pinned Vite override keeps stable VitePress on a patched
version compatible with its Vue plugin.

The Development section includes the repository's `CONTRIBUTING.md` and
`CHANGELOG.md` directly. Edit the root contributing guide to update its page;
Versionary generates the changelog. Repository links in these pages point to
GitHub, and links between them stay on the site.

## Published URLs and Markdown

The site retains `.html` chapter URLs, the print and contents views, and the
historical benchmarks and suppression redirects. `tests/legacy-routes.json`
records headings from the last mdBook build. When the heading sluggers differ,
`scripts/anchors.mjs` supplies aliases beside the new headings. Redirects retain
queries; browsers retain fragments when the redirect target supplies none.

The former `getting-started.html` installation catalogue redirects to
`installation.html`, keeping its section fragments. The project quickstart lives
at `guide/getting-started.html`.

Every content page has a Markdown variant under `/_markdown/`. Requests to HTML
pages with `Accept: text/markdown` receive that variant, including GET and HEAD
requests. Both representations send `Vary: Accept`. Markdown responses discard
HTML validators and byte ranges. The home export includes its hero and feature
text, and benchmark exports include the measurement tables.

```sh
curl -i -H 'Accept: text/markdown' https://arity.cc/
curl -i -H 'Accept: text/markdown' https://arity.cc/guide/editors.html
curl -i https://arity.cc/guide/editors.html
```

## Deployment and first cutover

The Documentation workflow builds and checks pull requests without deployment
credentials. Release tags (`v*`) and manual runs deploy the checked artifact.
Production deployments are serialized. The deploy job alone receives the
`CLOUDFLARE_ACCOUNT_ID` and `CLOUDFLARE_API_TOKEN` repository secrets.

`wrangler.toml` retains the existing Worker name and changes its routing from a
GitHub Pages overlay (`arity.cc/*`) to a Custom Domain (`arity.cc`). Before the
first production deployment:

1. Record the existing `arity.cc` DNS records, Worker routes, and deployment so
   the cutover can be reversed.
2. Ensure the token can deploy Workers and manage the Custom Domain in the
   account that owns the `arity.cc` zone. Creating the Worker may require
   Workers Admin access; attaching the hostname also needs zone access.
3. Remove only DNS records for the old GitHub Pages origin that conflict with
   the Custom Domain. Run the Documentation workflow from the migration commit.
4. Verify the live homepage, deep links, redirects and fragments, Markdown,
   installers, schema, and missing-page 404s. Then remove any obsolete
   `arity.cc/*` overlay route and disable GitHub Pages for this repository.

If domain attachment fails, restore the recorded DNS and routing before
retrying. Do not remove the previous origin until the new deployment is
verified. A local build or deployment dry run does not change production.
