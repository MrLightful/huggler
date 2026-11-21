# Huggler documentation site

Use Node.js 22 or later to build the Fumadocs site.

```sh
cd docs-app
npm ci
npm run dev
```

Open http://localhost:3000. Run `npm run typecheck` to check the TypeScript and `npm run build` to compile the MDX and export the site to `out/`.

Serve `out/` at the root of a static HTTP host with directory index support. To preview the export locally:

```sh
python3 -m http.server 3000 --directory out
```

The build generates `search-index.json` for browser-side search. Node is a build and development dependency for this site.

## Content structure

- `content/docs/concepts/` explains the runtime model and recovery behavior.
- `content/docs/guides/` covers workflows in terms shared across bindings.
- `content/docs/reference/` documents providers and the agent tool protocol.
- `content/docs/bindings/<language>/` contains each language’s overview, quickstart, API reference, and implementation guides.

To add a binding, create a folder with an `index.mdx` whose title is the language name and whose description identifies its runtime or package. Add a `meta.json` to order its pages. Use `quickstart.mdx` and `api.mdx` for the tutorial and API reference. Match existing topic filenames for equivalent workflows. The directory and language links discover the overview and matching pages automatically.

Update the capability table in `content/docs/bindings/index.mdx` from the binding’s public API. Only add topic pages the binding can document. On a binding page, the language navigation links to the equivalent topic where it exists and to the overview otherwise.

When moving a published page, add the old and new paths to `lib/legacy-routes.ts`. The static export renders the article at the old URL so existing heading links still work. Search indexes the new URL.
