import '@fontsource-variable/instrument-sans';
import './global.css';
import type { Metadata } from 'next';
import type { ReactNode } from 'react';
import { RootProvider } from 'fumadocs-ui/provider/next';
import SearchDialog from './search';

export const metadata: Metadata = {
  title: { default: 'Huggler', template: '%s · Huggler' },
  description: 'Documentation for embedding the Huggler Rust crate.',
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <body>
        <RootProvider search={{ SearchDialog }}>{children}</RootProvider>
      </body>
    </html>
  );
}
