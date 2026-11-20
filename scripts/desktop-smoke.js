// Installed only in the qualification build, alongside the unchanged desktop UI.
// Drive DOM controls and quit after the rendered conversation matches each step.
(async () => {
  const el = id => document.getElementById(id);
  const pause = () => new Promise(resolve => setTimeout(resolve, 50));
  async function until(test) {
    while (!test()) {
      const error = el("error").textContent;
      if (error || el("status").dataset.status === "failed") {
        throw Error(error || el("status").textContent);
      }
      await pause();
    }
  }
  const first = "provider thread resume fixture first turn complete";
  const second = "provider thread resume fixture second turn complete";
  const answers = () => [...document.querySelectorAll("article[data-role=assistant] p")].map(p => p.textContent);
  const step = Number(localStorage.getItem("smokeStep") || 1);
  if (step > 3) return;
  if (step === 2) {
    await until(() => !el("conversation").hidden && answers()[0] === first);
  } else {
    if (step === 3) {
      await until(() => answers()[1] === first + "\n" + second);
      el("new").click();
    }
    el("root").value = SMOKE_WORKSPACE;
    el("name").value = step === 1 ? "Resume check" : "Independent thread";
    el("launch").requestSubmit();
    await until(() => !el("conversation").hidden && el("status").dataset.status === "ready");
    if (answers().length !== 0) throw Error("New conversation contains messages");
  }
  const prompt = step === 2
    ? "Using the conversation history available in this resumed thread, first repeat the exact final answer you gave in the previous turn. Then on a new line write exactly: " + second
    : "Respond with exactly: " + first;
  el("prompt").value = prompt;
  el("send").requestSubmit();
  const expected = step === 2 ? first + "\n" + second : first;
  await until(() => answers().at(-1) === expected && el("status").dataset.status === "completed");
  if (step === 3 && answers().length !== 1) throw Error("Sibling view received unrelated messages");
  localStorage.setItem("smokeStep", String(step + 1));
  el("quit").click();
})().catch(error => { document.getElementById("error").textContent = String(error); });
