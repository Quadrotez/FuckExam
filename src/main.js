const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const isChatWindow = window.__TAURI__.window?.getCurrentWindow?.().label === 'chat';

const $ = (id) => document.getElementById(id);
let runtimePlatform = null;

async function checkPlatformAtStartup() {
    try {
        runtimePlatform = await invoke('platform_info');
        document.documentElement.dataset.os = runtimePlatform.os;
        if (!runtimePlatform.recorder) {
            applyMobileUi();
        }
        console.info('[fuckexam] platform:', runtimePlatform);
    } catch (e) {
        console.error('[fuckexam] platform check failed:', e);
    }
}

/** Android: умеет только смотреть и писать в чат, поэтому сразу открываем зрителя. */
function applyMobileUi() {
    document.documentElement.dataset.mobile = 'true';
    document.querySelector('.role-card[data-role="recorder"]')?.remove();
    document.querySelectorAll('[data-desktop-only]').forEach((el) => el.remove());
    const hint = document.querySelector('#view-landing .hint');
    if (hint) hint.textContent = 'просмотр трансляции и чат';
    const subtitle = document.querySelector('#view-landing .subtitle');
    if (subtitle) subtitle.textContent = 'Трансляция · Чат';
    if (!isChatWindow) showView('viewer');
}

/* ---------------- view router ---------------- */

const views = ['landing', 'recorder', 'settings', 'viewer'];

function showView(name) {
    views.forEach((v) => $('view-' + v).classList.toggle('active', v === name));
    if (name === 'recorder') {
        stopTimer();
        lastResult = '';
        refresh();
    }
    if (name === 'viewer') {
        maybeReconnect();
    }
}

document.querySelectorAll('.role-card').forEach((card) => {
    card.addEventListener('click', () => showView(card.dataset.role));
});

document.querySelectorAll('[data-back]').forEach((b) => {
    b.addEventListener('click', () => showView(b.dataset.back));
});

/* ---------------- recorder / recording ---------------- */

const btn = $('btn');
const statusTxt = $('status-text');
const statusBar = document.querySelector('.status-bar');
const timerEl = $('timer');
const windowsEl = $('windows');
const pathHint = $('path-hint');

let running = false;
let startedAt = null;
let tickHandle = null;
let lastResult = '';
let lastFiles = [];

function setStatus(text, recording) {
    statusTxt.textContent = text;
    statusBar.classList.toggle('recording', !!recording);
}

function formatTime(ms) {
    const total = Math.floor(ms / 1000);
    const m = String(Math.floor(total / 60)).padStart(2, '0');
    const s = String(total % 60).padStart(2, '0');
    return `${m}:${s}`;
}

function tick() {
    if (!startedAt) return;
    timerEl.textContent = formatTime(Date.now() - startedAt);
}

function startTimer() {
    startedAt = Date.now();
    tick();
    timerEl.classList.remove('hidden');
    tickHandle = setInterval(tick, 1000);
}

function stopTimer() {
    startedAt = null;
    if (tickHandle) {
        clearInterval(tickHandle);
        tickHandle = null;
    }
    timerEl.classList.add('hidden');
    timerEl.textContent = '';
}

function renderWindows(list) {
    if (list && list.length) {
        lastFiles = list.map((w) => w.output);
        windowsEl.innerHTML = '';
        list.forEach((w, i) => {
            const div = document.createElement('div');
            div.className = 'win-item';
            div.style.animationDelay = `${i * 70}ms`;
            const res =
                w.width > 0 && w.height > 0
                    ? `<span class="res">${w.width}×${w.height}</span>`
                    : '';
            div.innerHTML = `<span class="node">node ${w.nodeId}</span>${res}
                             <div class="path">${w.output}</div>`;
            windowsEl.appendChild(div);
        });
        if (lastFiles.length) {
            pathHint.textContent = lastFiles.join('\n');
        }
    } else if (list !== undefined) {
        windowsEl.innerHTML = '';
    }
}

function setReady(isRunning) {
    running = isRunning;
    btn.classList.toggle('recording', isRunning);
    btn.disabled = false;
    btn.querySelector('.btn-label').textContent = isRunning
        ? 'Остановить запись'
        : 'Начать запись';
}

async function refresh() {
    try {
        const snap = await invoke('recording_status');
        setReady(snap.running);
        renderWindows(snap.windows);
        if (snap.error) {
            setStatus('ошибка: ' + snap.error, false);
            return;
        }
        if (snap.running) {
            setStatus(`запись · ${snap.windows.length} окон`, true);
            startTimer();
        } else {
            stopTimer();
            if (lastResult) {
                setStatus(lastResult, false);
            } else {
                setStatus('готово', false);
            }
        }
    } catch (e) {
        setStatus('ошибка: ' + String(e), false);
    }
}

async function handleClick() {
    btn.classList.add('flicker');
    setTimeout(() => btn.classList.remove('flicker'), 500);

    try {
        if (running) {
            setStatus('остановка…', true);
            btn.disabled = true;
            stopTimer();
            const msg = await invoke('stop_recording');
            lastResult = msg;
            await refresh();
        } else {
            lastResult = '';
            setStatus('выберите окна…', false);
            btn.disabled = true;
            await invoke('start_recording');
            renderWindows((await invoke('recording_status')).windows);
            await refresh();
        }
    } catch (e) {
        setStatus('ошибка: ' + String(e), false);
        await refresh();
    }
}

btn.addEventListener('click', handleClick);

/* ---------------- settings / streaming ---------------- */

const sModeWrap = $('seg-mode');
const sPort = $('s-port');
const sRelayUrl = $('s-relay-url');
const sFps = $('s-fps');
const sQuality = $('s-quality');
const sMaxw = $('s-maxw');
const sStart = $('s-start');
const sStop = $('s-stop');
const sStatusEl = $('s-status');
const sRelayCheck = $('s-relay-check');
const sRelayCheckStatus = $('s-relay-check-status');
const sSave = $('s-save');
const sSaveFeedback = $('s-save-feedback');
const sWebLaunch = $('s-web-launch');
const sCloseTray = $('s-close-tray');
const sHideyMargin = $('s-hidey-margin');
const sHideTray = $('s-hide-tray');
const sHideAll = $('s-hide-all');
const sShowWindow = $('s-show-window');
const webLinksEl = $('web-links');
const webCopy = $('web-copy');
const webCopyAdmin = $('web-copy-admin');

const VIEWER_PREFS_KEY = 'fuckexam-viewer-prefs';
const VIEWER_SESSION_KEY = 'fuckexam-viewer-session';
const VIEWER_SESSION_TTL_MS = 30 * 60 * 1000;
const DEFAULT_SETTINGS = {
    port: 7335,
    fps: 24,
    quality: 75,
    maxWidth: 1280,
    relayUrl: '',
    mode: 'local',
    webOnLaunch: true,
    closeToTray: true,
    hideyMargin: 10,
};

let appSettings = { ...DEFAULT_SETTINGS };
let webInfo = { running: false, port: 7335, addresses: [] };
let sMode = 'local';
let sPollTimer = null;

function setSaveFeedback(text, kind = '') {
    sSaveFeedback.textContent = text;
    sSaveFeedback.classList.toggle('error', kind === 'error');
    sSaveFeedback.classList.toggle('success', kind === 'success');
}

function setSeg(mode) {
    sMode = mode === 'relay' ? 'relay' : 'local';
    sModeWrap.querySelectorAll('.seg-btn').forEach((b) => {
        b.classList.toggle('active', b.dataset.mode === sMode);
    });
    const relayOnly = sMode === 'relay';
    document.querySelectorAll('.s-field[data-field]').forEach((f) => {
        const isPort = f.dataset.field === 'port';
        f.style.display = relayOnly === isPort ? 'none' : 'flex';
    });
}

sModeWrap.querySelectorAll('.seg-btn').forEach((b) => {
    b.addEventListener('click', () => {
        setSeg(b.dataset.mode);
        renderSStatus(null);
    });
});

function normalizeRelayUrl(raw) {
    const value = String(raw || '').trim();
    if (!value) return '';
    return value.startsWith('ws://') || value.startsWith('wss://') ? value : 'ws://' + value;
}

function applySettingsToUi(s) {
    if (!s) return;
    sPort.value = s.port;
    sFps.value = String(s.fps);
    sQuality.value = String(s.quality);
    sMaxw.value = String(s.maxWidth);
    sRelayUrl.value = s.relayUrl || '';
    sWebLaunch.checked = s.webOnLaunch !== false;
    sCloseTray.checked = s.closeToTray !== false;
    if (sHideyMargin) sHideyMargin.value = String(s.hideyMargin ?? DEFAULT_SETTINGS.hideyMargin);
    setSeg(s.mode);
}

function collectSettings() {
    return {
        port: parseInt(sPort.value, 10) || DEFAULT_SETTINGS.port,
        fps: parseInt(sFps.value, 10) || DEFAULT_SETTINGS.fps,
        quality: parseInt(sQuality.value, 10) || DEFAULT_SETTINGS.quality,
        maxWidth: parseInt(sMaxw.value, 10) || DEFAULT_SETTINGS.maxWidth,
        relayUrl: normalizeRelayUrl(sRelayUrl.value),
        mode: sMode,
        webOnLaunch: sWebLaunch.checked,
        closeToTray: sCloseTray.checked,
        hideyMargin: sHideyMargin
            ? Math.max(0, parseInt(sHideyMargin.value, 10) || 0)
            : appSettings.hideyMargin,
    };
}

async function loadAppSettings() {
    try {
        appSettings = await invoke('get_settings');
    } catch (e) {
        console.error('[fuckexam] settings load failed:', e);
        appSettings = { ...DEFAULT_SETTINGS };
    }
    applySettingsToUi(appSettings);
    await refreshWebInfo();
}

async function saveAppSettings() {
    const saved = await invoke('save_settings', { settings: collectSettings() });
    appSettings = saved;
    applySettingsToUi(saved);
    await refreshWebInfo();
    return saved;
}

async function refreshWebInfo() {
    try {
        webInfo = await invoke('web_info');
    } catch (_) {
        return;
    }
    renderWebLinks();
}

function renderWebLinks() {
    if (!webLinksEl) return;
    const links = (webInfo.addresses || []).filter((a) => !a.includes('/admin'));
    const admin = (webInfo.addresses || []).find((a) => a.includes('/admin')) || '';
    const items = [];
    links.forEach((a) => items.push(a));
    if (admin) items.push(admin);
    webLinksEl.innerHTML = items.length
        ? items
              .map(
                  (a) =>
                      `<div class="s-addr"><code>${escapeHtml(a)}</code>` +
                      `<button class="btn-mini" data-copy="${escapeHtml(a)}">Копировать</button></div>`
              )
              .join('')
        : '<div class="s-sec">веб-сервер выключен</div>';
    if (!webInfo.running) {
        webLinksEl.innerHTML += '<div class="s-sec">сервер поднимется при запуске приложения или трансляции</div>';
    }
    wireCopyButtons(webLinksEl);
}

async function writeClipboard(text) {
    if (navigator.clipboard && navigator.clipboard.writeText) {
        try {
            await navigator.clipboard.writeText(text);
            return true;
        } catch (_) {}
    }
    try {
        const ta = document.createElement('textarea');
        ta.value = text;
        document.body.appendChild(ta);
        ta.select();
        document.execCommand('copy');
        ta.remove();
        return true;
    } catch (_) {
        return false;
    }
}

function wireCopyButtons(root) {
    root.querySelectorAll('[data-copy]').forEach((button) => {
        button.addEventListener('click', async () => {
            const done = await writeClipboard(String(button.dataset.copy));
            const original = button.textContent;
            button.textContent = done ? 'Готово' : 'Не вышло';
            setTimeout(() => (button.textContent = original), 1200);
        });
    });
    root.querySelectorAll('[data-copy-all]').forEach((button) => {
        button.addEventListener('click', async () => {
            const [addr, room] = String(button.dataset.copyAll).split('|');
            const done = await writeClipboard(`${addr}\nкомната: ${room}`);
            button.textContent = done ? 'Готово' : 'Не вышло';
            setTimeout(() => (button.textContent = 'Копировать'), 1200);
        });
    });
}

sSave.addEventListener('click', async () => {
    setSaveFeedback('сохранение…');
    try {
        await saveAppSettings();
        setSaveFeedback('сохранено', 'success');
    } catch (e) {
        setSaveFeedback(String(e), 'error');
    }
});

sHideTray.addEventListener('click', async () => {
    try {
        await saveAppSettings();
        await invoke('window_to_tray');
    } catch (e) {
        setSaveFeedback(String(e), 'error');
    }
});

sShowWindow.addEventListener('click', async () => {
    try {
        await invoke('window_show');
        setSaveFeedback('окно показано', 'success');
    } catch (e) {
        setSaveFeedback(String(e), 'error');
    }
});

sHideAll.addEventListener('click', async () => {
    const ok = confirm(
        'Скрыть окно и иконку трея? Вернуться можно только через веб-админку на этом компьютере.'
    );
    if (!ok) return;
    try {
        await saveAppSettings();
        await invoke('window_hide_all');
    } catch (e) {
        setSaveFeedback(String(e), 'error');
    }
});

if (webCopy) {
    webCopy.addEventListener('click', () => {
        const link = (webInfo.addresses || []).find((a) => !a.includes('/admin'));
        if (link) writeClipboard(link);
    });
}
if (webCopyAdmin) {
    webCopyAdmin.addEventListener('click', () => {
        const admin = (webInfo.addresses || []).find((a) => a.includes('/admin'));
        if (admin) writeClipboard(admin);
    });
}

function sTickerStart() {
    if (sPollTimer) return;
    sPollTimer = setInterval(async () => {
        try {
            renderSStatus(await invoke('stream_status'));
        } catch (_) {}
    }, 1500);
}

function sTickerStop() {
    if (sPollTimer) {
        clearInterval(sPollTimer);
        sPollTimer = null;
    }
}

function renderSStatus(st) {
    if (st && st.active) {
        let html = '';
        if (st.mode === 'local') {
            if (st.addresses && st.addresses.length) {
                html += '<div class="s-sec">Приложение/Android подключается по адресу:</div>';
                html += '<div class="s-addrs">';
                st.addresses.forEach((a) => {
                    html +=
                        `<div class="s-addr"><code>${escapeHtml(a)}</code>` +
                        `<button class="btn-mini" data-copy="${escapeHtml(a)}">Копировать</button></div>`;
                });
                html += '</div>';
            }
            if (st.web && st.web.length) {
                html += '<div class="s-sec">Браузер:</div>';
                html += '<div class="s-addrs">';
                st.web.forEach((a) => {
                    html +=
                        `<div class="s-addr"><code>${escapeHtml(a)}</code>` +
                        `<button class="btn-mini" data-copy="${escapeHtml(a)}">Копировать</button></div>`;
                });
                html += '</div>';
            }
        } else {
            html += '<div class="s-sec">Подключение к relay:</div>';
            html += '<div class="s-addrs">';
            html +=
                `<div class="s-addr"><code>${escapeHtml(st.relay || '')}</code>` +
                `<button class="btn-mini" data-copy="${escapeHtml(st.relay || '')}">Копировать</button></div>`;
            if (!st.room) {
                html += '<div class="s-sec">Комната создаётся на сервере…</div>';
            }
            if (st.room) {
                html += '<div class="s-sec">Укажите зрителю:</div>';
                html +=
                    `<div class="s-addr s-room"><code>${escapeHtml(st.room)}</code>` +
                    `<button class="btn-mini" data-copy="${escapeHtml(st.room)}">Копировать комнату</button></div>`;
                html +=
                    `<div class="s-addr s-copy-all"><span>Скопировать всё</span>` +
                    `<button class="btn-mini" data-copy-all="${escapeHtml(st.relay || '')}|${escapeHtml(st.room)}">Копировать</button></div>`;
            }
            html += '</div>';
        }
        html +=
            `<div class="s-sec">В сети: <b>${st.viewers}</b> зрителей · ` +
            `стримов: ${st.streams.length} · кадров: ${st.frames ?? 0}</div>`;
        sStatusEl.innerHTML = html;
        sStatusEl.classList.remove('hidden');
        wireCopyButtons(sStatusEl);
        sStart.disabled = true;
        sStop.disabled = false;
        sTickerStart();
    } else {
        sStatusEl.classList.add('hidden');
        sStatusEl.innerHTML = '';
        sStart.disabled = false;
        sStop.disabled = true;
        sTickerStop();
    }
}

async function streamStart() {
    try {
        const settings = await saveAppSettings();
        const options = {
            fps: settings.fps,
            quality: settings.quality,
            maxWidth: settings.maxWidth,
        };
        let st;
        if (settings.mode === 'relay') {
            if (!settings.relayUrl) {
                setStatus('трансляция: укажите адрес relay в настройках', false);
                return;
            }
            st = await invoke('relay_connect', { url: settings.relayUrl, options });
        } else {
            st = await invoke('stream_start', { options });
        }
        renderSStatus(st);
        await refreshWebInfo();
    } catch (e) {
        setStatus('трансляция: ' + String(e), false);
    }
}

async function streamStop() {
    try {
        await invoke('stream_stop');
        renderSStatus(null);
        setStatus('трансляция остановлена', false);
    } catch (e) {
        setStatus('трансляция: ' + String(e), false);
    }
}

sStart.addEventListener('click', streamStart);
sStop.addEventListener('click', streamStop);

sRelayCheck.addEventListener('click', async () => {
    const url = normalizeRelayUrl(sRelayUrl.value);
    if (!url) {
        sRelayCheckStatus.textContent = 'введите адрес relay';
        sRelayCheckStatus.classList.remove('ok');
        sRelayCheckStatus.classList.add('err');
        return;
    }
    sRelayUrl.value = url;
    sRelayCheck.disabled = true;
    sRelayCheckStatus.textContent = 'проверка…';
    sRelayCheckStatus.classList.remove('ok', 'err');
    try {
        sRelayCheckStatus.textContent = '✓ ' + (await invoke('relay_ping', { url }));
        sRelayCheckStatus.classList.add('ok');
    } catch (e) {
        sRelayCheckStatus.textContent = '✗ ' + String(e);
        sRelayCheckStatus.classList.add('err');
    } finally {
        sRelayCheck.disabled = false;
    }
});

listen('stream-room', () => {
    invoke('stream_status').then(renderSStatus).catch(() => {});
});
listen('stream-error', (evt) => {
    setStatus('трансляция: ' + String(evt.payload ?? 'ошибка relay'), false);
});

const recChatLog = $('chat-log');
const recChatForm = $('rec-chat-form');
const recChatInput = $('rec-chat-input');
const recChatMarkdown = $('rec-chat-markdown');
const recChatAttach = $('rec-chat-attach');
const recChatImageInput = $('rec-chat-image-input');
const recChatImagePreview = $('rec-chat-image-preview');
const recChatImagePreviewImg = $('rec-chat-image-preview-img');
const recChatImagePreviewName = $('rec-chat-image-preview-name');
const recChatImageRemove = $('rec-chat-image-remove');
const recChatFeedback = $('rec-chat-feedback');
const recChatPopout = $('rec-chat-popout');

const MAX_CHAT_TEXT_LENGTH = 4000;
const MAX_CHAT_IMAGE_BYTES = 512 * 1024;
const MAX_CHAT_IMAGE_BASE64_LENGTH = 4 * Math.ceil(MAX_CHAT_IMAGE_BYTES / 3);
let pendingRecorderChatImage = null;
let chatHistoryPending = isChatWindow;
let chatEventsDuringHistory = [];

function isValidChatImage(image) {
    if (!image || typeof image.data !== 'string' || typeof image.name !== 'string') return false;
    if (!image.data.length || image.data.length > MAX_CHAT_IMAGE_BASE64_LENGTH || image.data.length % 4 !== 0) return false;
    if (!/^[A-Za-z0-9+/]+={0,2}$/.test(image.data)) return false;
    try {
        const bytes = atob(image.data);
        return bytes.length <= MAX_CHAT_IMAGE_BYTES && bytes.length >= 4 &&
            bytes.charCodeAt(0) === 0xff && bytes.charCodeAt(1) === 0xd8 &&
            bytes.charCodeAt(bytes.length - 2) === 0xff && bytes.charCodeAt(bytes.length - 1) === 0xd9;
    } catch (_) {
        return false;
    }
}

function appendChatMessage(log, msg) {
    const text = typeof msg.text === 'string' ? msg.text : '';
    const hasImage = isValidChatImage(msg.image);
    if (!text && !hasImage) return;

    const div = document.createElement('div');
    div.className = 'msg';

    const from = document.createElement('span');
    from.className = 'from';
    from.textContent = String(msg.from || 'зритель');
    div.appendChild(from);

    if (text) {
        const body = document.createElement('span');
        body.className = 'text';
        if (msg.markdown === true && typeof window.marked?.parse === 'function' &&
            typeof window.DOMPurify?.sanitize === 'function') {
            try {
                const html = window.marked.parse(text, { gfm: true, breaks: true });
                body.classList.add('markdown');
                body.innerHTML = window.DOMPurify.sanitize(html, {
                    ALLOWED_TAGS: ['p', 'br', 'strong', 'em', 'del', 'blockquote', 'ul', 'ol', 'li', 'code', 'pre', 'a', 'h1', 'h2', 'h3', 'h4', 'hr', 'table', 'thead', 'tbody', 'tr', 'th', 'td'],
                    ALLOWED_ATTR: ['href', 'title'],
                });
                decorateCodeBlocks(body);
                body.querySelectorAll('a[href]').forEach((link) => {
                    link.target = '_blank';
                    link.rel = 'noopener noreferrer';
                });
            } catch (err) {
                console.warn('[fuckexam] Markdown rendering failed:', err);
                body.textContent = text;
            }
        } else {
            if (msg.markdown === true) {
                console.error('[fuckexam] Markdown was requested but Marked/DOMPurify is unavailable.');
            }
            body.textContent = text;
        }
        div.appendChild(body);
    }

    if (hasImage) {
        const image = document.createElement('img');
        image.className = 'chat-image';
        image.loading = 'lazy';
        image.alt = msg.image.name.slice(0, 120);
        image.src = `data:image/jpeg;base64,${msg.image.data}`;
        div.appendChild(image);
    }

    log.appendChild(div);
    log.scrollTop = log.scrollHeight;
    while (log.children.length > 100) log.removeChild(log.firstChild);
}

function escapeHtml(s) {
    return String(s).replace(/[&<>"']/g, (c) =>
        ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c])
    );
}

/** Блоки кода: подсветка, нумерация строк и перенос без повторения номера. */
function decorateCodeBlocks(root) {
    root.querySelectorAll('pre > code').forEach((code) => {
        const text = code.textContent.replace(/\n$/, '');
        const lines = text.split('\n');
        const fragment = document.createDocumentFragment();
        lines.forEach((line, index) => {
            const row = document.createElement('span');
            row.className = 'code-line';

            const number = document.createElement('span');
            number.className = 'code-no';
            number.textContent = String(index + 1);
            number.setAttribute('aria-hidden', 'true');

            const source = document.createElement('span');
            source.className = 'code-src';
            source.textContent = line.length ? line : '\u00a0';

            row.append(number, source);
            fragment.appendChild(row);
        });
        code.textContent = '';
        code.appendChild(fragment);
        code.parentElement?.classList.add('code-block');
    });
}

const chatEventSubscription = listen('stream-chat', (evt) => {
    const payload = evt.payload;
    let msg = null;
    if (typeof payload === 'string') {
        try {
            const parsed = JSON.parse(payload);
            msg = parsed && parsed.from ? parsed : { from: '?', text: payload };
        } catch (_) {
            msg = { from: '?', text: payload };
        }
    } else if (payload && payload.from) {
        msg = payload;
    }
    if (!msg) return;
    if (chatHistoryPending) chatEventsDuringHistory.push(msg);
    else appendChatMessage(recChatLog, msg);
});

/* ---------------- viewer ---------------- */

const vUrl = $('v-url');
const vRoom = $('v-room');
const vName = $('v-name');
const vConnect = $('v-connect');
const vDisconnect = $('v-disconnect');
const vStatusText = $('v-status-text');
const vGrid = $('v-grid');
const vChatLog = $('v-chat-log');
const vChatForm = $('v-chat-form');
const vChatInput = $('v-chat-input');
const vChatMarkdown = $('v-chat-markdown');
const vChatImageInput = $('v-chat-image-input');
const vChatAttach = $('v-chat-attach');
const vChatImagePreview = $('v-chat-image-preview');
const vChatImagePreviewImg = $('v-chat-image-preview-img');
const vChatImagePreviewName = $('v-chat-image-preview-name');
const vChatImageRemove = $('v-chat-image-remove');
const vChatFeedback = $('v-chat-feedback');

let vsock = null;
let vFrames = 0;
let vConnAborted = false;
let vServerError = '';
let pendingChatImage = null;
let vReconnectTimer = null;
let vReconnectDelay = 1500;
let vUserClosed = false;
const vstreams = new Map(); // node -> { img, stale }

function vSetStatus(text) {
    vStatusText.textContent = text;
}

function setChatFeedback(text, kind = '') {
    vChatFeedback.textContent = text;
    vChatFeedback.classList.toggle('error', kind === 'error');
    vChatFeedback.classList.toggle('success', kind === 'success');
}

function clearPendingChatImage() {
    pendingChatImage = null;
    vChatImagePreviewImg.removeAttribute('src');
    vChatImagePreviewName.textContent = '';
    vChatImagePreview.classList.add('hidden');
}

function imageBlobToDataUrl(blob) {
    return new Promise((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => typeof reader.result === 'string' ? resolve(reader.result) : reject(new Error('не удалось прочитать фото'));
        reader.onerror = () => reject(new Error('не удалось прочитать фото'));
        reader.readAsDataURL(blob);
    });
}

async function decodeChatImage(file) {
    if (typeof createImageBitmap === 'function') {
        try {
            return await createImageBitmap(file);
        } catch (_) {
            // Older WebKitGTK versions may expose createImageBitmap without
            // supporting every image format; retry through a regular Image.
        }
    }

    const url = URL.createObjectURL(file);
    return new Promise((resolve, reject) => {
        const image = new Image();
        image.onload = () => resolve({
            width: image.naturalWidth,
            height: image.naturalHeight,
            image,
            close: () => URL.revokeObjectURL(url),
        });
        image.onerror = () => {
            URL.revokeObjectURL(url);
            reject(new Error('не удалось открыть фото в этом браузере'));
        };
        image.src = url;
    });
}

async function prepareChatImage(file) {
    const supportedTypes = ['image/jpeg', 'image/png', 'image/webp', 'image/gif', 'image/bmp', 'image/avif'];
    const mimeType = String(file.type || '').toLowerCase();
    const supportedName = /\.(?:jpe?g|png|webp|gif|bmp|avif)$/i.test(String(file.name || ''));
    if (!supportedTypes.includes(mimeType) && !supportedName) {
        throw new Error('Выберите JPEG, PNG, WebP, GIF, BMP или AVIF-фото.');
    }
    if (file.size > 15 * 1024 * 1024) throw new Error('Исходный файл больше 15 МБ.');

    const bitmap = await decodeChatImage(file);
    try {
        if (bitmap.width * bitmap.height > 30_000_000) {
            throw new Error('Размер фото превышает 30 мегапикселей.');
        }
        const scale = Math.min(1, 1600 / Math.max(bitmap.width, bitmap.height));
        const canvas = document.createElement('canvas');
        canvas.width = Math.max(1, Math.round(bitmap.width * scale));
        canvas.height = Math.max(1, Math.round(bitmap.height * scale));
        const context = canvas.getContext('2d');
        if (!context) throw new Error('Обработка фото недоступна в этом браузере.');
        context.drawImage(bitmap.image || bitmap, 0, 0, canvas.width, canvas.height);

        let blob = null;
        for (const quality of [0.82, 0.72, 0.62, 0.52, 0.42]) {
            blob = await new Promise((resolve) => canvas.toBlob(resolve, 'image/jpeg', quality));
            if (!blob) throw new Error('Не удалось обработать фото в этом браузере.');
            if (blob.size <= MAX_CHAT_IMAGE_BYTES) break;
        }
        if (!blob || blob.size > MAX_CHAT_IMAGE_BYTES) {
            throw new Error('Не удалось сжать фото до 512 КБ. Выберите изображение поменьше.');
        }

        const dataUrl = await imageBlobToDataUrl(blob);
        const baseName = String(file.name || 'photo').replace(/[\\/\0-\x1f\x7f]/g, '_').replace(/\.[^.]*$/, '').slice(0, 90) || 'photo';
        return { name: `${baseName}.jpg`, data: dataUrl.slice(dataUrl.indexOf(',') + 1) };
    } finally {
        bitmap.close?.();
    }
}

function vEnsureImg(node, w, h) {
    let entry = vstreams.get(node);
    if (!entry) {
        const wrap = document.createElement('div');
        wrap.className = 'v-cell';
        wrap.innerHTML = `<span class="v-node">node ${node}</span>`;
        wrap.title = 'клик — полный экран';
        wrap.addEventListener('click', () => {
            if (document.fullscreenElement === wrap) {
                document.exitFullscreen().catch(() => {});
            } else {
                wrap.requestFullscreen().catch(() => {});
            }
        });
        const canvas = document.createElement('canvas');
        canvas.width = 640;
        canvas.height = 360;
        wrap.appendChild(canvas);
        vGrid.appendChild(wrap);
        entry = { canvas, wrap, rendering: false, pending: null };
        vstreams.set(node, entry);
    }
    if (w && h) {
        const cw = Math.min(w, 1920);
        const ch = Math.max(1, Math.round((h / w) * cw));
        if (entry.canvas.width !== cw || entry.canvas.height !== ch) {
            entry.canvas.width = cw;
            entry.canvas.height = ch;
        }
        entry.wrap.style.aspectRatio = `${w} / ${h}`;
    }
    return entry;
}

function vRemoveImg(node) {
    const entry = vstreams.get(node);
    if (entry) {
        entry.wrap.remove();
        vstreams.delete(node);
    }
}

function vReconcile(streams) {
    const ids = new Set(streams.map((s) => s.id));
    for (const [node, entry] of vstreams.entries()) {
        if (!ids.has(node)) vRemoveImg(node);
    }
    streams.forEach((s) => vEnsureImg(s.id, s.w, s.h));
}

// «только последний кадр»: декодим не более одного JPEG за раз на окно,
// чтобы очередь декодирования не отставала (иначе задержка секундами).
function vFeedFrame(entry, jpeg) {
    if (entry.rendering) {
        entry.pending = jpeg;
        return;
    }
    entry.rendering = true;
    const img = new Image();
    const url = URL.createObjectURL(new Blob([jpeg], { type: 'image/jpeg' }));
    img.onload = () => {
        try {
            entry.canvas.getContext('2d').drawImage(img, 0, 0, entry.canvas.width, entry.canvas.height);
        } catch (_) {}
        URL.revokeObjectURL(url);
        entry.rendering = false;
        if (entry.pending) {
            const p = entry.pending;
            entry.pending = null;
            vFeedFrame(entry, p);
        }
    };
    img.onerror = () => {
        URL.revokeObjectURL(url);
        entry.rendering = false;
        if (entry.pending) {
            const p = entry.pending;
            entry.pending = null;
            vFeedFrame(entry, p);
        }
    };
    img.src = url;
}

function vHandleBinary(buf) {
    if (buf.byteLength < 5) return;
    const dv = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
    const node = dv.getUint32(0, true);
    const jpeg = buf.slice(4);
    const entry = vEnsureImg(node);
    vFrames++;
    if (vFrames % 30 === 0) {
        vSetStatus(`получено кадров: ${vFrames}`);
    }
    vFeedFrame(entry, jpeg);
}

function vHandleBytes(buf) {
    // бинарный кадр [u32LE node] + jpeg
    try {
        vHandleBinary(buf);
    } catch (e) {
        if (window.__TAURI__) vSetStatus('ошибка кадра: ' + String(e));
    }
}

function vHandleMsg(evt) {
    if (typeof evt.data === 'string') {
        let msg;
        try {
            msg = JSON.parse(evt.data);
        } catch (_) {
            return;
        }
        switch (msg.type) {
            case 'welcome':
                vFrames = 0;
                vChatLog.innerHTML = '';
                vReconcile(msg.streams || []);
                if (!msg.streams || msg.streams.length === 0) {
                    vSetStatus('подключено, стримов нет — ждите записывающего или проверьте комнату');
                } else {
                    vSetStatus(`подключено (${msg.streams.length} стримов)`);
                }
                if (msg.history) {
                    msg.history.forEach((m) => appendChatMessage(vChatLog, m));
                }
                break;
            case 'streams':
                vReconcile(msg.streams || []);
                if (!msg.streams || msg.streams.length === 0) {
                    vSetStatus('подключено, стримов нет');
                } else {
                    vSetStatus(`подключено (${msg.streams.length} стримов)`);
                }
                break;
            case 'stream-end':
                vRemoveImg(msg.id);
                break;
            case 'chat':
                if (msg.from) appendChatMessage(vChatLog, msg);
                break;
            case 'error':
                vServerError = String(msg.error || 'ошибка сервера');
                vSetStatus('ошибка: ' + vServerError);
                break;
        }
        return;
    }
    const d = evt.data;
    if (typeof ArrayBuffer !== 'undefined' && d instanceof ArrayBuffer) {
        vHandleBytes(new Uint8Array(d));
    } else if (typeof Blob !== 'undefined' && d instanceof Blob) {
        d.arrayBuffer().then((ab) => vHandleBytes(new Uint8Array(ab))).catch(() => {});
    } else if (ArrayBuffer.isView(d)) {
        vHandleBytes(new Uint8Array(d.buffer, d.byteOffset, d.byteLength));
    }
}

function loadViewerPrefs() {
    try {
        const raw = localStorage.getItem(VIEWER_PREFS_KEY);
        if (!raw) return;
        const prefs = JSON.parse(raw);
        if (prefs.url) vUrl.value = prefs.url;
        if (prefs.room) vRoom.value = prefs.room;
        if (prefs.name) vName.value = prefs.name;
    } catch (_) {}
}

function saveViewerPrefs() {
    try {
        localStorage.setItem(
            VIEWER_PREFS_KEY,
            JSON.stringify({
                url: vUrl.value.trim(),
                room: vRoom.value.trim(),
                name: vName.value.trim(),
            })
        );
    } catch (_) {}
}

function saveViewerSession() {
    try {
        localStorage.setItem(
            VIEWER_SESSION_KEY,
            JSON.stringify({
                url: vUrl.value.trim(),
                room: vRoom.value.trim(),
                name: vName.value.trim(),
                savedAt: Date.now(),
            })
        );
    } catch (_) {}
}

function touchViewerSession() {
    if (vsock || readViewerSession()) saveViewerSession();
}

function dropViewerSession() {
    try {
        localStorage.removeItem(VIEWER_SESSION_KEY);
    } catch (_) {}
}

function readViewerSession() {
    try {
        const raw = localStorage.getItem(VIEWER_SESSION_KEY);
        if (!raw) return null;
        const session = JSON.parse(raw);
        if (!session || !session.url) return null;
        if (
            typeof session.savedAt !== 'number' ||
            Date.now() - session.savedAt > VIEWER_SESSION_TTL_MS
        ) {
            dropViewerSession();
            return null;
        }
        return session;
    } catch (_) {
        return null;
    }
}

function viewerActive() {
    return !isChatWindow && $('view-viewer').classList.contains('active');
}

function maybeReconnect() {
    if (!viewerActive() || vUserClosed) return;
    if (vsock && vsock.readyState !== WebSocket.CLOSED) return;
    const session = readViewerSession();
    if (!session) return;
    if (!vUrl.value.trim()) vUrl.value = session.url;
    if (!vRoom.value.trim() && session.room) vRoom.value = session.room;
    if (!vName.value.trim() && session.name) vName.value = session.name;
    vReconnectDelay = 1500;
    doConnect({ auto: true });
}

function scheduleReconnect() {
    clearTimeout(vReconnectTimer);
    if (vUserClosed || !readViewerSession()) return;
    const delay = vReconnectDelay;
    vReconnectDelay = Math.min(vReconnectDelay * 2, 15000);
    vReconnectTimer = setTimeout(() => {
        if (document.hidden) return;
        maybeReconnect();
    }, delay);
}

function doConnect(options = {}) {
    if (vsock && vsock.readyState !== WebSocket.CLOSED) return;
    let url = vUrl.value.trim();
    if (!url) {
        vSetStatus('введите адрес ws://…');
        return;
    }
    if (!url.startsWith('ws://') && !url.startsWith('wss://')) {
        url = 'ws://' + url;
        vUrl.value = url;
    }
    let ws;
    try {
        ws = new WebSocket(url);
    } catch (e) {
        vSetStatus('неверный адрес (нужен формат ws://ip:port)');
        return;
    }
    clearTimeout(vReconnectTimer);
    vUserClosed = false;
    vConnAborted = false;
    vServerError = '';
    vsock = ws;
    ws.binaryType = 'arraybuffer';
    vSetStatus(options.auto ? 'переподключение…' : 'подключение…');
    vConnect.disabled = true;
    vDisconnect.disabled = false;

    const connTimer = setTimeout(() => {
        if (ws.readyState === WebSocket.CONNECTING) {
            vConnAborted = true;
            vSetStatus('ошибка: таймаут подключения (проверьте адрес и фаервол сервера)');
            try {
                ws.close();
            } catch (_) {}
            if (vsock === ws) {
                vsock = null;
                vConnect.disabled = false;
                vDisconnect.disabled = true;
                scheduleReconnect();
            }
        }
    }, 8000);

    ws.addEventListener('open', () => {
        clearTimeout(connTimer);
        vReconnectDelay = 1500;
        saveViewerPrefs();
        saveViewerSession();
        ws.send(
            JSON.stringify({
                type: 'hello',
                role: 'viewer',
                room: (vRoom.value.trim() || '').toUpperCase(),
            })
        );
    });
    ws.addEventListener('message', vHandleMsg);
    ws.addEventListener('close', () => {
        clearTimeout(connTimer);
        if (vsock !== ws) return;
        vResetConn();
        if (vUserClosed) {
            vSetStatus('отключено');
            return;
        }
        if (vConnAborted) return;
        vSetStatus(
            vServerError
                ? 'ошибка: ' + vServerError
                : 'соединение потеряно, переподключаемся…'
        );
        saveViewerSession();
        scheduleReconnect();
    });
    ws.addEventListener('error', () => {
        clearTimeout(connTimer);
        if (!vConnAborted) {
            vSetStatus('ошибка соединения');
        }
    });
}

function vResetConn() {
    vsock = null;
    vConnect.disabled = false;
    vDisconnect.disabled = true;
    vFrames = 0;
    for (const node of [...vstreams.keys()]) vRemoveImg(node);
}

function doDisconnect() {
    vUserClosed = true;
    clearTimeout(vReconnectTimer);
    dropViewerSession();
    if (vsock) {
        try {
            vsock.close();
        } catch (_) {}
    }
    vResetConn();
    vFrames = 0;
    vSetStatus('отключено');
}

vConnect.addEventListener('click', () => doConnect());
vDisconnect.addEventListener('click', doDisconnect);

// Android сворачивает WebView и теряет WebSocket: при возврате в приложение
// молча восстанавливаем соединение, если сессия ещё не протухла.
document.addEventListener('visibilitychange', () => {
    if (!viewerActive()) return;
    if (document.hidden) {
        touchViewerSession();
        return;
    }
    maybeReconnect();
});
window.addEventListener('online', maybeReconnect);
window.addEventListener('focus', maybeReconnect);
if (!isChatWindow) {
    listen('app-resumed', maybeReconnect);
}

vChatAttach.addEventListener('click', () => vChatImageInput.click());
vChatImageInput.addEventListener('change', async () => {
    const file = vChatImageInput.files?.[0];
    vChatImageInput.value = '';
    if (!file) return;
    vChatAttach.disabled = true;
    setChatFeedback('Обработка фото…');
    try {
        pendingChatImage = await prepareChatImage(file);
        vChatImagePreviewImg.src = `data:image/jpeg;base64,${pendingChatImage.data}`;
        vChatImagePreviewName.textContent = pendingChatImage.name;
        vChatImagePreview.classList.remove('hidden');
        setChatFeedback('Фото готово к отправке.');
    } catch (e) {
        clearPendingChatImage();
        setChatFeedback(String(e), 'error');
    } finally {
        vChatAttach.disabled = false;
    }
});
vChatImageRemove.addEventListener('click', () => {
    clearPendingChatImage();
    setChatFeedback('Фото удалено.');
});
vChatInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.ctrlKey && !e.shiftKey && !e.metaKey && !e.isComposing) {
        e.preventDefault();
        vChatForm.requestSubmit();
    }
});

vChatForm.addEventListener('submit', (e) => {
    e.preventDefault();
    const text = vChatInput.value.trim();
    if (!text && !pendingChatImage) return;
    if (text.length > MAX_CHAT_TEXT_LENGTH) {
        setChatFeedback(`Сообщение слишком длинное (максимум ${MAX_CHAT_TEXT_LENGTH} символов).`, 'error');
        return;
    }
    if (!vsock || vsock.readyState !== WebSocket.OPEN) {
        setChatFeedback('Сначала подключитесь к трансляции.', 'error');
        return;
    }
    const message = {
        type: 'chat',
        from: vName.value.trim() || 'зритель',
        text,
        markdown: vChatMarkdown.checked,
        image: pendingChatImage,
    };
    try {
        vsock.send(JSON.stringify(message));
        vChatInput.value = '';
        clearPendingChatImage();
        saveViewerPrefs();
        saveViewerSession();
        setChatFeedback('Сообщение отправлено.', 'success');
    } catch (err) {
        setChatFeedback('Не удалось отправить сообщение: ' + String(err), 'error');
    }
});

function setRecorderChatFeedback(text, kind = '') {
    recChatFeedback.textContent = text;
    recChatFeedback.classList.toggle('error', kind === 'error');
    recChatFeedback.classList.toggle('success', kind === 'success');
}

function clearPendingRecorderChatImage() {
    pendingRecorderChatImage = null;
    recChatImagePreviewImg.removeAttribute('src');
    recChatImagePreviewName.textContent = '';
    recChatImagePreview.classList.add('hidden');
}

recChatPopout.addEventListener('click', async () => {
    try {
        await invoke('open_chat_window');
    } catch (err) {
        setRecorderChatFeedback('Не удалось открыть отдельный чат: ' + String(err), 'error');
    }
});

recChatAttach.addEventListener('click', () => recChatImageInput.click());
recChatImageInput.addEventListener('change', async () => {
    const file = recChatImageInput.files?.[0];
    recChatImageInput.value = '';
    if (!file) return;
    recChatAttach.disabled = true;
    setRecorderChatFeedback('Обработка фото…');
    try {
        pendingRecorderChatImage = await prepareChatImage(file);
        recChatImagePreviewImg.src = `data:image/jpeg;base64,${pendingRecorderChatImage.data}`;
        recChatImagePreviewName.textContent = pendingRecorderChatImage.name;
        recChatImagePreview.classList.remove('hidden');
        setRecorderChatFeedback('Фото готово к отправке.');
    } catch (err) {
        clearPendingRecorderChatImage();
        setRecorderChatFeedback(String(err), 'error');
    } finally {
        recChatAttach.disabled = false;
    }
});
recChatImageRemove.addEventListener('click', () => {
    clearPendingRecorderChatImage();
    setRecorderChatFeedback('Фото удалено.');
});
recChatInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.ctrlKey && !e.shiftKey && !e.metaKey && !e.isComposing) {
        e.preventDefault();
        recChatForm.requestSubmit();
    }
});
recChatForm.addEventListener('submit', async (e) => {
    e.preventDefault();
    const text = recChatInput.value.trim();
    if (!text && !pendingRecorderChatImage) return;
    if (text.length > MAX_CHAT_TEXT_LENGTH) {
        setRecorderChatFeedback(`Сообщение слишком длинное (максимум ${MAX_CHAT_TEXT_LENGTH} символов).`, 'error');
        return;
    }

    recChatInput.disabled = true;
    recChatAttach.disabled = true;
    $('rec-chat-send').disabled = true;
    try {
        await invoke('chat_send', {
            text,
            markdown: recChatMarkdown.checked,
            image: pendingRecorderChatImage,
        });
        recChatInput.value = '';
        clearPendingRecorderChatImage();
        setRecorderChatFeedback('Сообщение отправлено.', 'success');
    } catch (err) {
        setRecorderChatFeedback(String(err), 'error');
    } finally {
        recChatInput.disabled = false;
        recChatAttach.disabled = false;
        $('rec-chat-send').disabled = false;
    }
});

function chatMessageKey(msg) {
    const image = msg.image ? [msg.image.name, msg.image.data] : null;
    return JSON.stringify([msg.from, msg.text, msg.markdown === true, image]);
}

async function populateDetachedChatHistory() {
    try {
        await chatEventSubscription;
        const history = await invoke('chat_history');
        const snapshotCounts = new Map();
        history.forEach((msg) => {
            appendChatMessage(recChatLog, msg);
            const key = chatMessageKey(msg);
            snapshotCounts.set(key, (snapshotCounts.get(key) || 0) + 1);
        });
        chatEventsDuringHistory.forEach((msg) => {
            const key = chatMessageKey(msg);
            const count = snapshotCounts.get(key) || 0;
            if (count > 0) snapshotCounts.set(key, count - 1);
            else appendChatMessage(recChatLog, msg);
        });
    } catch (err) {
        console.error('[fuckexam] chat history load failed:', err);
        chatEventsDuringHistory.forEach((msg) => appendChatMessage(recChatLog, msg));
    } finally {
        chatEventsDuringHistory = [];
        chatHistoryPending = false;
    }
}

/* ---------------- boot ---------------- */

loadViewerPrefs();
void loadAppSettings();
checkPlatformAtStartup();
if (isChatWindow) {
    document.documentElement.dataset.chatWindow = 'true';
    showView('recorder');
    void populateDetachedChatHistory();
} else {
    refresh();
    showView('landing');
}
