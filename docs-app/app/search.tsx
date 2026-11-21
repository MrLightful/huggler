'use client';

import SearchDialog, { type DefaultSearchDialogProps } from 'fumadocs-ui/components/dialog/search-default';

export default function DocsSearch(props: DefaultSearchDialogProps) {
  return <SearchDialog {...props} type="static" api="/search-index.json" />;
}
