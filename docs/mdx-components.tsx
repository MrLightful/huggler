import { BindingDirectory, BindingLinks } from '@/components/binding-links';
import defaultComponents from 'fumadocs-ui/mdx';
import type { MDXComponents } from 'mdx/types';

export function useMDXComponents(components?: MDXComponents): MDXComponents {
  return { ...defaultComponents, BindingDirectory, BindingLinks, ...components };
}
