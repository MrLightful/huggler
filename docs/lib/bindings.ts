import { source } from './source';

export function getBindings(topic: readonly string[] = []) {
  return source.getPages()
    .filter(({ slugs }) => slugs[0] === 'bindings' && slugs.length === 2)
    .map((overview) => {
      const page = source.getPage([...overview.slugs, ...topic]);

      return { overview, page };
    })
    .sort((a, b) => a.overview.data.title.localeCompare(b.overview.data.title));
}
