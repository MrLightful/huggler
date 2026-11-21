declare const brand: unique symbol
type Id<Name extends string> = string & { readonly [brand]: Name }
export type ProviderInstanceId = Provider | Id<"ProviderInstance">
export type TaskId = Id<"Task">
export type NativeThreadId = Id<"NativeThread">
export type NativeTurnId = Id<"NativeTurn">
export type EnvironmentId = Id<"Environment">
export type CommandId = Id<"Command">
export type ThreadId = Id<"Thread">
export type ProjectId = Id<"Project">
export type RunId = Id<"Run">
export type MessageId = Id<"Message">
export type RequestId = Id<"Request">
export type EventCursor = Id<"EventCursor">
export type HistoryCursor = Id<"HistoryCursor">

export const providerInstanceId = (value: string) => value as ProviderInstanceId
export const commandId = (value: string) => value as CommandId
export const threadId = (value: string) => value as ThreadId
export const runId = (value: string) => value as RunId
export const projectId = (value: string) => value as ProjectId
export const messageId = (value: string) => value as MessageId
export const requestId = (value: string) => value as RequestId
export const eventCursor = (value: string) => value as EventCursor
export const historyCursor = (value: string) => value as HistoryCursor

export type Provider = "codex" | "claudeAgent" | "cursor" | "grok" | "fx" | "kiro" |
  "antigravity" | "acpRegistry" | "opencode" | "pi" | "muse"

export interface ProviderInstance {
  id: ProviderInstanceId
  driver: Provider
  executable: string
  args?: string[]
  home?: string | null
  piExtension?: string | null
  serverUrl?: string | null
  passwordEnv?: string | null
  requestApprovals?: boolean
  /** Allow workspace writes for Codex outside plan mode. Defaults to false. */
  workspaceWrite?: boolean
}

export interface OpenOptions {
  stateDir: string
  instances?: ProviderInstance[]
  storage?: {
    databasePath?: string
    worktreeRoot?: { kind: "shared" | "projectRelative"; path: string }
    worktreeName?: string
  }
  output?: {
    name?: string
    title?: string
    version?: string
    instructions?: string
    providerLabels?: Record<string, string>
    tools?: Record<string, { name?: string; title?: string; description?: string }>
  }
}

export interface ProviderReport {
  version: string | null
  transport: string
  authenticated: boolean | null
  models: string[]
  resume: boolean
  agentTools: boolean
}
export interface ProviderUsage {
  providerInstanceId: ProviderInstanceId
  reportedRuns: string
  inputTokens: string
  outputTokens: string
}
export interface Project { id: ProjectId; root: string }
export interface ThreadSummary { id: ThreadId; name: string }
export interface ThreadCommand { commandId: CommandId; threadId: ThreadId }
export interface LaunchOptions extends ThreadCommand {
  projectId: ProjectId
  name: string
  providerInstanceId: ProviderInstanceId
  model?: string | null
}
export interface SendOptions extends ThreadCommand { text: string; mode?: "default" | "plan" }
export interface Receipt extends ThreadCommand {
  messageId: MessageId | null
  runId: RunId | null
  eventSequences: EventCursor[]
}
export interface SendReceipt extends Receipt { messageId: MessageId; runId: RunId }

export type RunStatus = "queued" | "starting" | "running" | "waiting" | "interrupted" | "completed" | "failed"
interface RunFields {
  id: RunId
  ordinal: string
  queueHeld: boolean
  deliverySlot: string | null
  nativeTurn: NativeTurnId | null
  usageLimitResetAt: string | null
  promptSuggestion: string | null
  error: string | null
}
export type Run = { [S in RunStatus]: RunFields & { status: S } }[RunStatus]

export interface Message {
  id: MessageId
  runId: RunId
  role: "user" | "assistant"
  text: string
  origin: { environmentId: EnvironmentId | null; threadId: ThreadId; runId: RunId; requestKey: string } | null
}
export interface PermissionRequest {
  id: RequestId
  runId: RunId
  nativeChild: { threadId: NativeThreadId; turnId: NativeTurnId | null } | null
  toolCall: unknown
  options: { optionId: string; name: string; kind: string }[]
}
export interface QuestionRequest {
  id: RequestId
  runId: RunId
  questions: {
    id: string
    header?: string
    question?: string
    isOther: boolean
    isSecret: boolean
    options: { label: string; description?: string; preview?: string; [key: string]: unknown }[] | null
    [key: string]: unknown
  }[]
}
export interface RequestOwner { threadId: ThreadId; runId: RunId; requestId: RequestId }

export interface NativeChild {
  runId: RunId
  parentThreadId: NativeThreadId
  parentTurnId: NativeTurnId | null
  threadId: NativeThreadId
  turnId: NativeTurnId | null
  path: string
  canInterrupt: boolean
  status: "starting" | "running" | "requires_action" | "idle" | "unknown" |
    "completed" | "interrupted" | "failed" | "disconnected"
  result: string | null
}
export interface DelegatedTask {
  taskId: TaskId
  parentThreadId: ThreadId
  parentRunId: RunId
  childThreadId: ThreadId
  childRunId: RunId
  resultRunId: RunId | null
  result: string | null
  status: RunStatus
  delivery: "pending" | "acknowledged" | "delivered" | "disposed"
  wakeRunId: RunId | null
}
export interface Thread extends ThreadSummary {
  projectId: ProjectId
  provider: Provider
  providerInstanceId: ProviderInstanceId
  worktreePath: string | null
  model: string | null
  nativeThread: NativeThreadId | null
  messages: Message[]
  runs: Run[]
  permissions: PermissionRequest[]
  questions: QuestionRequest[]
  nativeChildren: NativeChild[]
  tasks: DelegatedTask[]
  cursor: EventCursor
}
export interface HistoryOptions { before?: HistoryCursor; query?: string; limit?: number }
export interface ConversationDetail extends ThreadSummary {
  providerInstanceId: ProviderInstanceId
  model: string | null
  latestRun: Run | null
  foregroundRun: Run | null
  queuedCount: string
  queueHeld: boolean
  streamingText: string
  commandItems: string[]
  questions: QuestionRequest[]
  permissions: PermissionRequest[]
  nativeChildren: NativeChild[]
  history: {
    messages: { message: Message; truncated: boolean }[]
    older: HistoryCursor | null
    sequence: EventCursor
  }
}
export type Change =
  | { type: "launched" }
  | { type: "sessionReset" }
  | { type: "run"; run: Run }
  | { type: "message"; message: Message }
  | { type: "bound"; nativeThread: NativeThreadId }
  | { type: "modelSelected"; model: string | null }
  | { type: "questionRequested"; request: QuestionRequest }
  | { type: "permissionRequested"; request: PermissionRequest }
  | { type: "questionClosed"; id: RequestId; answered: boolean; responderRun: RunId | null }
  | { type: "permissionClosed"; id: RequestId; option: string | null }
  | { type: "nativeChild"; child: NativeChild }
  | { type: "task"; task: DelegatedTask }
  | { type: "commandOutputAvailable"; runId: RunId; item: string }
  | { type: "usageLimitContinuation"; failed: RunId; runId: RunId }
  | { type: "steer"; runId: RunId; commandId: CommandId; outcome: "pending" | "accepted" | "uncertain" | { rejected: string } }
  | { type: "runStopRequested"; runId: RunId }
  | { type: "outputDelta"; runId: RunId; text: string }
  | { type: "userNotice"; runId: RunId; text: string }
  | { type: "peerCreated"; callerRunId: RunId; peerThreadId: ThreadId }
  | { type: "runInterruptRequested"; runId: RunId; callerThreadId: ThreadId; callerRunId: RunId; reason: string | null }

export interface ThreadEvent { sequence: EventCursor; threadId: ThreadId; change: Change }
export interface Subscription {
  snapshot: Thread
  events: AsyncIterableIterator<ThreadEvent> & { return(): Promise<IteratorResult<ThreadEvent>> }
}
