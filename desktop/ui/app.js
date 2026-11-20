const { invoke, Channel } = window.__TAURI__.core;
const el = id => document.getElementById(id);
const uuid = () => crypto.randomUUID();
let current;
let generation = 0;
let busy = false;

function report(error) {
  const detail = error?.detail;
  if (error?.code === "conflict") {
    el("error").textContent = `Command already used in conversation ${detail.original_thread}.`;
  } else {
    el("error").textContent = typeof detail === "string" ? detail : error?.code || String(error);
  }
}
function render() {
  if (!current) return;
  const run = current.runs.at(-1);
  el("messages").replaceChildren(...current.messages.map(message => {
    const row = document.createElement("article");
    row.dataset.role = message.role;
    const who = document.createElement("strong");
    who.textContent = message.role === "assistant" ? (current.provider === "claudeAgent" ? "Claude" : "Codex") : "You";
    const text = document.createElement("p");
    text.textContent = message.text;
    row.append(who, text);
    return row;
  }));
  el("permissions").replaceChildren(...current.permissions.map(request => {
    const thread = current.id;
    const panel = document.createElement("fieldset");
    const title = document.createElement("legend");
    title.textContent = request.tool_call.title || "Tool permission";
    const details = document.createElement("pre");
    details.textContent = JSON.stringify(request.tool_call.rawInput ?? request.tool_call, null, 2);
    panel.append(title, details);
    for (const option of request.options) {
      const button = document.createElement("button");
      button.type = "button";
      button.dataset.kind = option.kind;
      button.textContent = option.name;
      button.onclick = async () => {
        panel.disabled = true;
        try {
          await invoke("respond_to_permission", {thread, run: request.run_id, request: request.id, option: option.optionId});
        } catch (error) { report(error); panel.disabled = false; }
      };
      panel.append(button);
    }
    return panel;
  }));
  el("status").textContent = run?.error || run?.status || "Ready";
  el("status").dataset.status = run?.status || "ready";
  el("submit").disabled = busy || (run && !["completed", "failed","interrupted"].includes(run.status));
}
async function view(id) {
  const version = ++generation;
  current = null;
  el("submit").disabled = true;
  el("messages").replaceChildren();
  el("permissions").replaceChildren();
  el("status").textContent = "";
  el("status").dataset.status = "";
  const channel = new Channel();
  channel.onmessage = frame => {
    if (version !== generation) return;
    if (frame.type === "snapshot") current = frame.value;
    else {
      const { thread_id, change } = frame.value;
      if (!current || current.id !== thread_id) return;
      if (change.type === "message") current.messages.push(change.message);
      if (change.type === "permission_requested") current.permissions.push(change.request);
      if (change.type === "permission_closed") current.permissions = current.permissions.filter(p => p.id !== change.id);
      if (change.type === "run") {
        const index = current.runs.findIndex(run => run.id === change.run.id);
        if (index < 0) current.runs.push(change.run);
        else current.runs[index] = change.run;
      }
    }
    render();
  };
  await invoke("subscribe", { thread: id, channel });
  if (version !== generation) return;
  localStorage.setItem("selected", id);
  el("threads").value = id;
  el("conversation").hidden = false;
  el("launch").hidden = true;
}
async function load(selected) {
  const threads = await invoke("threads");
  el("threads").replaceChildren(...threads.map(t => new Option(t.name, t.id)));
  el("threads").disabled = !threads.length;
  if (threads.length) await view(threads.find(t => t.id === selected)?.id || threads[0].id);
}
function bind(id, type, handler) {
  el(id).addEventListener(type, async event => {
    event.preventDefault();
    el("error").textContent = "";
    try { await handler(); } catch (error) { report(error); }
  });
}
bind("new", "click", async () => {
  ++generation;
  current = null;
  el("conversation").hidden = true;
  el("threads").selectedIndex = -1;
  el("launch").hidden = false;
  el("name").value = "";
  await invoke("unsubscribe");
  el("root").focus();
});
bind("launch", "submit", async () => {
  const project = await invoke("open_project", { root: el("root").value });
  const id = uuid();
  await invoke("dispatch", { command: { id: uuid(), thread_id: id,
    action: { type: "launch_provider", provider: el("provider").value, project: project.id, name: el("name").value, model: null } } });
  await load(id);
});
bind("threads", "change", () => view(el("threads").value));
bind("send", "submit", async () => {
  if (!current || el("submit").disabled) return;
  busy = true;
  render();
  try {
    await invoke("dispatch", { command: { id: uuid(), thread_id: current.id,
      action: { type: "send", text: el("prompt").value } } });
    el("prompt").value = "";
  } finally { busy = false; render(); }
});
bind("quit", "click", () => invoke("quit"));
load(localStorage.getItem("selected")).catch(report);
