import Link from 'next/link';
import { getBindings } from '@/lib/bindings';

export function BindingLinks({ topic }: { topic: string }) {
  return (
    <ul className="binding-links">
      {getBindings([topic]).map(({ overview, page }) => {
        if (!page) return null;

        return <li key={overview.url}><Link href={page.url}>{overview.data.title}</Link></li>;
      })}
    </ul>
  );
}

export function BindingDirectory() {
  return (
    <ul className="binding-directory">
      {getBindings(['quickstart']).map(({ overview, page }) => (
        <li key={overview.url}>
          <div>
            <Link href={overview.url} className="binding-name">{overview.data.title}</Link>
            <p>{overview.data.description}</p>
          </div>
          {page && <Link href={page.url} aria-label={page.data.title}>Quickstart <span aria-hidden="true">→</span></Link>}
        </li>
      ))}
    </ul>
  );
}

export function BindingNavigation({ slugs }: { slugs: string[] }) {
  const [section, id, ...topic] = slugs;
  if (section !== 'bindings' || !id) return null;

  return (
    <nav aria-label="Programming language" className="binding-navigation">
      <span>Language</span>
      <ul>
        {getBindings(topic).map(({ overview, page }) => {
          const current = overview.slugs[1] === id;

          return (
            <li key={overview.url}>
              <Link
                href={(page ?? overview).url}
                aria-current={current ? 'page' : undefined}
                aria-label={page ? undefined : `${overview.data.title} overview`}
              >
                {overview.data.title}{!page && <span className="binding-overview"> overview</span>}
              </Link>
            </li>
          );
        })}
      </ul>
    </nav>
  );
}
