# Huggler documentation site

Use Node.js 22 or later to build the Fumadocs site.

```sh
cd docs-app
npm ci
npm run dev
```

Open http://localhost:3000. Pages are in `content/docs`. The `meta.json` files set sidebar order. Run `npm run typecheck` to check the TypeScript and `npm run build` to compile the MDX and export the site to `out/`.

Serve `out/` at the root of a static HTTP host with directory index support. To preview the export locally:

```sh
python3 -m http.server 3000 --directory out
```

The build generates `search-index.json` for browser-side search. Node is a build and development dependency for this site.
