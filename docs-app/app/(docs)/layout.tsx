import { source } from '@/lib/source';
import { DocsLayout } from 'fumadocs-ui/layouts/docs';
import type { ReactNode } from 'react';

export default function Layout({ children }: { children: ReactNode }) {
  return (
    <DocsLayout
      tree={source.getPageTree()}
      nav={{
        title: (
          <span className="docs-brand">
            Huggler <span>Docs</span>
          </span>
        ),
      }}
      githubUrl="https://github.com/MrLightful/huggler"
    >
      {children}
    </DocsLayout>
  );
}
