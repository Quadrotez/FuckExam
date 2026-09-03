const api = globalThis.browser ?? globalThis.chrome;
const TASKS_PAGE = "tasks.html";

function queryActiveTab() {
  return api.tabs.query({ active: true, currentWindow: true });
}

async function scanActiveTab() {
  const [tab] = await queryActiveTab();
  if (!tab?.id || !/^https:\/\/(school|dnevnik)\.mos\.ru\//.test(tab.url || "")) {
    return { error: "Открой страницу МЭШ на school.mos.ru или dnevnik.mos.ru и повтори сканирование." };
  }
  try {
    return await api.tabs.sendMessage(tab.id, { type: "SCAN_PAGE" });
  } catch {
    return { error: "Не удалось подключиться к странице. Перезагрузи вкладку МЭШ и повтори попытку." };
  }
}

async function openTasks() {
  const url = api.runtime.getURL(TASKS_PAGE);
  const tabs = await api.tabs.query({ url });
  if (tabs[0]?.id) {
    await api.tabs.update(tabs[0].id, { active: true });
    return;
  }
  await api.tabs.create({ url });
}

api.runtime.onMessage.addListener((message) => {
  if (message?.type === "STORE_TASKS") {
    return api.storage.local.set({ tasks: message.tasks || [], source: message.source || "" });
  }
  if (message?.type === "SCAN_ACTIVE_TAB") return scanActiveTab();
  if (message?.type === "OPEN_TASKS") return openTasks();
  return undefined;
});

api.action.onClicked.addListener(async () => {
  const result = await scanActiveTab();
  if (!result?.error) await openTasks();
});
