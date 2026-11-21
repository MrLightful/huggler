import { existsSync } from "node:fs"
import { join } from "node:path"

export interface Cancellation { cancel(): void }
export interface NativeStream {
  next(cancel: Cancellation): Promise<string | null>
}
export interface NativeRuntime {
  close(): Promise<void>
  execute(request: string): Promise<string>
  probe(instance: string, cwd: string): Promise<string>
  answer(thread: string, run: string, request: string, answers: string): Promise<void>
  wait(thread: string, run: string, cancel: Cancellation): Promise<string | null>
  subscribe(thread: string, after?: string): Promise<{ snapshot: string; stream: NativeStream }>
}
interface Addon {
  NativeEnvironment: new () => { open(options: string): Promise<NativeRuntime> }
  NativeCancel: new () => Cancellation
}

function load(): Addon {
  const target: string = require("../platform.cjs")()

  // Present only in a source checkout after npm run build.
  const local = join(__dirname, "../npm", target, "huggler.node")
  if (existsSync(local)) return require(local)
  const name = `@huggler/sdk-${target}`
  let path: string
  try {
    path = require.resolve(name)
  } catch (cause) {
    throw new Error(`Missing Huggler binary package ${name}. Install the SDK with optional dependencies enabled.`, { cause })
  }
  return require(path)
}

export const addon = load()
export const environment = new addon.NativeEnvironment()
