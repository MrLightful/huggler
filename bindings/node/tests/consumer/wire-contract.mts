import type * as sdk from "@huggler/sdk"

// Native string IDs acquire a brand at the JS boundary. Keep literal unions
// such as run statuses intact while checking the underlying wire values.
type Wire<T> = T extends string
  ? T extends object ? string : T
  : T extends object ? { [K in keyof T]: Wire<T[K]> } : T
