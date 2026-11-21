import { BindingNavigation } from '@/components/binding-links';
import { legacyRoutes } from '@/lib/legacy-routes';
import { source } from '@/lib/source';
import { useMDXComponents } from '@/mdx-components';
import { DocsBody, DocsDescription, DocsPage, DocsTitle } from 'fumadocs-ui/page';
import { notFound } from 'next/navigation';

type Props = { params: Promise<{ slug?: string[] }> };

function resolvePage(slug: string[] = []) {
  const key = slug.join('/');
  const legacy = Object.hasOwn(legacyRoutes, key);
  const page = source.getPage(legacy ? legacyRoutes[key].split('/') : slug);
  if (!page) notFound();

  return { page, legacy };
}

export default async function Page({ params }: Props) {
  const { page, legacy } = resolvePage((await params).slug);

  const Content = page.data.body;
  return (
    <DocsPage toc={page.data.toc} footer={{ enabled: !legacy && page.slugs.length > 0 }}>
      <BindingNavigation slugs={page.slugs} />
      <DocsTitle>{page.data.title}</DocsTitle>
      <DocsDescription>{page.data.description}</DocsDescription>
      {legacy && <p className="legacy-location"><a href={page.url}>Open this page in the binding docs</a></p>}
      <DocsBody>
        <Content components={useMDXComponents()} />
      </DocsBody>
    </DocsPage>
  );
}

export function generateStaticParams() {
  return [...source.generateParams(), ...Object.keys(legacyRoutes).map((path) => ({ slug: path.split('/') }))];
}

export async function generateMetadata({ params }: Props) {
  const { page, legacy } = resolvePage((await params).slug);
  return {
    title: page.slugs.length === 0 ? { absolute: 'Huggler Docs' } : page.data.title,
    description: page.data.description,
    robots: legacy ? { index: false, follow: true } : undefined,
  };
}
