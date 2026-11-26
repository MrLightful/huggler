// Static hosts can serve the original article paths and their heading fragments.
export const legacyRoutes: Readonly<Record<string, string>> = {
  quickstart: 'bindings/rust/quickstart',
  'typescript-quickstart': 'bindings/typescript/quickstart',
  'reference/runtime-api': 'bindings/rust/api',
  'reference/typescript-sdk': 'bindings/typescript/api',
  'guides/connect-hosts': 'bindings/rust/connect-hosts',
  'guides/application-integrations': 'bindings/rust/application-integrations',
};
