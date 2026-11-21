import type { CommandId, ThreadId } from "./types"

type ErrorDetails =
  | { code: "conflict"; detail: { command: CommandId; originalThread: ThreadId; requestedThread: ThreadId } }
  | { code: "usage_limit"; detail: { message: string; resetAt: string | null } }
  | { code: "login_required" | "server_access_denied" | "already_answered" | "already_open" |
      "forbidden" | "interrupted" | "closed" | "cursor_expired" | "history_changed" | "busy"; detail: null }
  | { code: "checkout_unavailable" | "missing_executable" | "incompatible_provider" | "not_found" | "invalid" | "integration" |
      "unsupported" | "schema_version" | "storage" | "data" | "io" | "provider" | "worker"; detail: string }

// Distribute each code so Extract<ErrorInfo, { code: C }> works for every C.
export type ErrorInfo = {
  [C in ErrorDetails["code"]]: { code: C; detail: (ErrorDetails & { code: C })["detail"] }
}[ErrorDetails["code"]]

export function encode(value: unknown): string {
  try {
    const json = JSON.stringify(value)
    if (json !== undefined) return json
  } catch { /* Cycles, BigInts and user-defined serialization can fail. */ }
  throw new HugglerError({ code: "invalid", detail: "Expected JSON data" }, "Expected JSON data")
}

class SdkError extends Error {
  readonly code: ErrorInfo["code"]
  readonly detail: ErrorInfo["detail"]
  constructor(info: ErrorInfo, message: string) {
    super(message)
    this.name = "HugglerError"
    this.code = info.code
    this.detail = info.detail
  }
}

export type HugglerError = Error & ErrorInfo
export const HugglerError = SdkError as new (info: ErrorInfo, message: string) => HugglerError

export async function invoke<T>(call: () => Promise<T>): Promise<T> {
  try {
    return await call()
  } catch (cause) {
    if (!(cause instanceof Error) || cause instanceof HugglerError) throw cause
    if ("code" in cause && (cause.code === "StringExpected" || cause.code === "InvalidArg")) {
      throw new HugglerError({ code: "invalid", detail: "Invalid SDK argument" }, "Invalid SDK argument")
    }
    let payload
    try {
      payload = JSON.parse(cause.message)
    } catch {
      throw cause
    }
    if (!payload || typeof payload.code !== "string" || typeof payload.message !== "string") throw cause
    throw new HugglerError({ ...payload, detail: payload.detail ?? null }, payload.message)
  }
}
