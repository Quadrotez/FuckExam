const api = globalThis.browser ?? globalThis.chrome;
const $ = (selector) => document.querySelector(selector);
const state = { tasks: [], source: "" };

function send(message) {
  return api.runtime.sendMessage(message);
}

function normalizeStatus(status) {
  const value = String(status ?? "").toLowerCase();
  return value.includes("done") || value.includes("выполн") || value.includes("заверш") ? "done" : "new";
}

function formatDate(value) {
  if (!value) return "";
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? String(value) : date.toLocaleDateString("ru-RU");
}

function render() {
  const query = $("#search").value.trim().toLowerCase();
  const status = $("#status").value;
  const filtered = state.tasks.filter((task) => {
    const haystack = [task.title, task.subject, task.description, ...(task.materials ?? []).map((item) => item.title)].join(" ").toLowerCase();
    return (!query || haystack.includes(query)) && (status === "all" || normalizeStatus(task.status) === status);
  });
  $("#stats").textContent = `Показано: ${filtered.length} из ${state.tasks.length}`;
  const container = $("#tasks");
  container.replaceChildren();
  if (!filtered.length) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = state.tasks.length ? "По выбранным фильтрам заданий нет." : "Задания не найдены. Открой страницу заданий МЭШ и запусти сканирование.";
    container.append(empty);
    return;
  }
  const template = $("#task-template");
  for (const task of filtered) {
    const fragment = template.content.cloneNode(true);
    fragment.querySelector(".subject").textContent = task.subject || "Без предмета";
    fragment.querySelector(".due-date").textContent = task.dueDate ? `до ${formatDate(task.dueDate)}` : "";
    fragment.querySelector(".title").textContent = task.title || "Без названия";
    const description = fragment.querySelector(".description");
    description.textContent = task.description || "";
    description.hidden = !task.description;
    const materials = fragment.querySelector(".materials");
    for (const material of task.materials ?? []) {
      const row = document.createElement("div");
      row.className = "material";
      const label = document.createElement("span");
      label.textContent = material.title || "Материал";
      row.append(label);
      if (material.url) {
        const link = document.createElement("a");
        link.href = material.url;
        link.target = "_blank";
        link.rel = "noreferrer";
        link.textContent = "Открыть";
        row.append(link);
      }
      materials.append(row);
    }
    const source = fragment.querySelector(".source-link");
    if (task.sourceUrl) source.href = task.sourceUrl;
    else source.remove();
    container.append(fragment);
  }
}

async function load() {
  const result = await api.storage.local.get(["tasks", "source"]);
  state.tasks = Array.isArray(result.tasks) ? result.tasks : [];
  state.source = result.source || "";
  $("#source").textContent = state.source ? `Источник: ${state.source}` : "Источник не выбран";
  render();
}

$("#search").addEventListener("input", render);
$("#status").addEventListener("change", render);
$("#clear").addEventListener("click", async () => {
  await api.storage.local.remove(["tasks", "source"]);
  await load();
});
$("#rescan").addEventListener("click", async () => {
  const response = await send({ type: "SCAN_ACTIVE_TAB" });
  if (response?.error) {
    $("#notice").hidden = false;
    $("#notice").textContent = response.error;
    return;
  }
  await load();
});

load().catch((error) => {
  $("#notice").hidden = false;
  $("#notice").textContent = `Не удалось загрузить задания: ${error.message}`;
});
