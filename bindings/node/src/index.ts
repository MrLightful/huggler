import { addAbortListener } from "node:events"
import { addon, environment, type NativeRuntime, type NativeStream } from "./native"
import { HugglerError, encode, invoke } from "./errors"
import type {
  ConversationDetail, EventCursor, HistoryOptions, LaunchOptions, Message, MessageId, OpenOptions,
  Project, ProviderInstance, ProviderInstanceId, ProviderReport, ProviderUsage, Receipt, RequestOwner, Run, RunId,
  SendOptions, SendReceipt, Subscription, Thread, ThreadCommand, ThreadEvent, ThreadId, ThreadSummary,
} from "./types"

export * from "./types"
export { HugglerError } from "./errors"
export type { ErrorInfo } from "./errors"

function events(stream: NativeStream | undefined, signal?: AbortSignal): Subscription["events"] {
  const cancel = new addon.NativeCancel()
  let pending = false
  return {
    [Symbol.asyncIterator]() { return this },
    async next(): Promise<IteratorResult<ThreadEvent>> {
      const source = stream
      if (!source) return { done: true, value: undefined }
      if (pending) throw new HugglerError({ code: "invalid", detail: "next already pending" }, "Await the previous next() first")
      pending = true
      const listener = signal && addAbortListener(signal, () => cancel.cancel())
      try {
        signal?.throwIfAborted()
        const value = await invoke(() => source.next(cancel))
        if (!stream) return { done: true, value: undefined }
        signal?.throwIfAborted()
        if (value == null) {
          stream = undefined
          return { done: true, value: undefined }
        }
        return { done: false, value: JSON.parse(value) }
      } catch (cause) {
        stream = undefined
        throw cause
      } finally {
        listener?.[Symbol.dispose]()
        pending = false
      }
    },
    async return() {
      stream = undefined
      cancel.cancel()
      return { done: true, value: undefined }
    },
  }
}

export class Huggler {
  #runtime: NativeRuntime
  #closing?: Promise<void>
  private constructor(runtime: NativeRuntime) { this.#runtime = runtime }

  static async open(options: OpenOptions): Promise<Huggler> {
    return new Huggler(await invoke(() => environment.open(encode(options))))
  }

  #active(): NativeRuntime {
    if (this.#closing) throw new HugglerError({ code: "closed", detail: null }, "Runtime is closed")
    return this.#runtime
  }

  async #execute<T>(op: string, args: object = {}): Promise<T> {
    return JSON.parse(await invoke(() => this.#active().execute(encode({ ...args, op }))))
  }

  openProject(path: string): Promise<Project> { return this.#execute("openProject", { path }) }
  listProviderInstances(): Promise<ProviderInstance[]> { return this.#execute("listProviderInstances") }
  readProviderUsage(): Promise<ProviderUsage[]> { return this.#execute("readProviderUsage") }
  async probeProvider(instance: ProviderInstanceId, cwd: string): Promise<ProviderReport> {
    return JSON.parse(await invoke(() => this.#active().probe(instance, cwd)))
  }
  launch(options: LaunchOptions): Promise<Receipt> { return this.#execute("launch", options) }
  send(options: SendOptions): Promise<SendReceipt> { return this.#execute("send", options) }
  stopThread(options: ThreadCommand): Promise<Receipt> { return this.#execute("stopThread", options) }
  listThreads(): Promise<ThreadSummary[]> { return this.#execute("listThreads") }
  readThread(threadId: ThreadId): Promise<Thread> { return this.#execute("readThread", { threadId }) }
  readHistory(threadId: ThreadId, { before, query = "", limit = 50 }: HistoryOptions = {}): Promise<ConversationDetail> {
    return this.#execute("readHistory", { threadId, before, query, limit })
  }
  readMessage(threadId: ThreadId, messageId: MessageId): Promise<Message> {
    return this.#execute("readMessage", { threadId, messageId })
  }

  async respondToPermission(options: RequestOwner & { option: string }): Promise<void> {
    await this.#execute("respondToPermission", options)
  }
  async respondToQuestion({ threadId, runId, requestId, answers }: RequestOwner & { answers: Record<string, string | string[]> }): Promise<void> {
    await invoke(() => this.#active().answer(threadId, runId, requestId, encode(answers)))
  }

  /** Cancelling a wait detaches this observer. Use stopThread to interrupt the run. */
  async wait(threadId: ThreadId, runId: RunId, { signal }: { signal?: AbortSignal } = {}): Promise<Run> {
    signal?.throwIfAborted()
    const cancel = new addon.NativeCancel()
    const listener = signal && addAbortListener(signal, () => cancel.cancel())
    try {
      const result = await invoke(() => this.#active().wait(threadId, runId, cancel))
      signal?.throwIfAborted()
      if (result == null) throw new Error("Run observation cancelled")
      return JSON.parse(result)
    } finally {
      listener?.[Symbol.dispose]()
    }
  }

  /** A supplied cursor replays strictly after that position, even if the snapshot already includes those events. */
  async subscribe(threadId: ThreadId, { after, signal }: { after?: EventCursor; signal?: AbortSignal } = {}): Promise<Subscription> {
    signal?.throwIfAborted()
    const { stream, snapshot } = await invoke(() => this.#active().subscribe(threadId, after))
    signal?.throwIfAborted()
    return { snapshot: JSON.parse(snapshot), events: events(stream, signal) }
  }

  close(): Promise<void> {
    return this.#closing ??= invoke(() => this.#runtime.close())
  }
}
