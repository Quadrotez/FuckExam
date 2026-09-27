const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
let runtimePlatform = null;

async function checkPlatformAtStartup() {
    try {
        runtimePlatform = await invoke('platform_info');
        document.documentElement.dataset.os = runtimePlatform.os;
        if (!runtimePlatform.recorder) {
            $('btn').disabled = true;
            $('path-hint').textContent = `Запись экранов не поддерживается на ОС: ${runtimePlatform.os}`;
        }
        console.info('[fuckexam] platform:', runtimePlatform);
    } catch (e) {
        console.error('[fuckexam] platform check failed:', e);
    }
}

/* ---------------- view router ---------------- */

const views = ['landing', 'recorder', 'viewer'];

function showView(name) {
    views.forEach((v) => $('view-' + v).classList.toggle('active', v === name));
    if (name === 'recorder') {
        stopTimer();
        lastResult = '';
        refresh();
    }
    if (name === 'viewer') {
        doDisconnect();
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

/* ---------------- recorder / streaming ---------------- */

const sModeWrap = $('seg-mode');
const sPort = $('s-port');
const sRelayUrl = $('s-relay-url');
const sRoom = $('s-room');
const sStart = $('s-start');
const sStop = $('s-stop');
const sStatusEl = $('s-status');
const sRelayCheck = $('s-relay-check');
const sRelayCheckStatus = $('s-relay-check-status');
const sStatusBox = document.querySelector('#view-recorder .status-bar');

const SETTINGS_KEY = 'fuckexam-settings';

function loadSettings() {
    try {
        const raw = localStorage.getItem(SETTINGS_KEY);
        if (!raw) return;
        const s = JSON.parse(raw);
        if (s.port) sPort.value = s.port;
        if (s.relayUrl) sRelayUrl.value = s.relayUrl;
        if (s.relayRoom) sRoom.value = s.relayRoom;
        if (s.viewerUrl) vUrl.value = s.viewerUrl;
        if (s.viewerRoom) vRoom.value = s.viewerRoom;
        if (s.viewerName) vName.value = s.viewerName;
    } catch (_) {}
}

function saveSettings() {
    try {
        localStorage.setItem(
            SETTINGS_KEY,
            JSON.stringify({
                port: sPort.value,
                relayUrl: sRelayUrl.value,
                relayRoom: sRoom.value,
                viewerUrl: vUrl.value,
                viewerRoom: vRoom.value,
                viewerName: vName.value,
            })
        );
    } catch (_) {}
}

let sMode = 'local';
let sActAddr = '';

function setSeg(mode) {
    sMode = mode;
    sModeWrap.querySelectorAll('.seg-btn').forEach((b) => {
        b.classList.toggle('active', b.dataset.mode === mode);
    });
    const relayOnly = mode === 'relay';
    document.querySelectorAll('.s-field').forEach((f) => {
        const isPort = f.dataset.field === 'port';
        f.style.display = relayOnly === isPort ? 'none' : 'flex';
    });
    sActAddr = '';
    renderSStatus(null);
}

sModeWrap.querySelectorAll('.seg-btn').forEach((b) => {
    b.addEventListener('click', () => {
        setSeg(b.dataset.mode);
        sActAddr = '';
        renderSStatus(null);
    });
});

let sPollTimer = null;

function sTickerStart() {
    if (sPollTimer) return;
    sPollTimer = setInterval(async () => {
        try {
            const st = await invoke('stream_status');
            renderSStatus(st);
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
        if (sMode === 'local' && st.addresses.length) {
            html += '<div class="s-sec">Зрители подключаются по адресу:</div>';
            html += '<div class="s-addrs">';
            st.addresses.forEach((a) => {
                html +=
                    `<div class="s-addr"><code>${escapeHtml(a)}</code>` +
                    `<button class="btn-mini" data-copy="${escapeHtml(a)}">Скопировать</button></div>`;
            });
            html += '</div>';
            sActAddr = st.addresses[0] || '';
        } else if (sMode === 'relay') {
            html += '<div class="s-sec">Подключение к relay:</div>';
            html += '<div class="s-addrs">';
            html +=
                `<div class="s-addr"><code>${escapeHtml(st.relay || '')}</code>` +
                `<button class="btn-mini" data-copy="${escapeHtml(st.relay || '')}">Скопировать</button></div>`;
            if (st.room) {
                html +=
                    `<div class="s-addr s-room"><code>${escapeHtml(st.room)}</code>` +
                    `<button class="btn-mini" data-copy="${escapeHtml(st.room)}">Скопировать</button></div>`;
            }
            html += '</div>';
            if (st.relay) {
                const roomTxt = st.room ? st.room : '';
                html += '<div class="s-sec">Укажите зрителю:</div>';
                html += '<div class="s-addrs">';
                html +=
                    `<div class="s-addr"><code>${escapeHtml(st.relay)}</code>` +
                    `<button class="btn-mini" data-copy="${escapeHtml(st.relay)}">Копировать адрес</button></div>`;
                if (roomTxt) {
                    html +=
                        `<div class="s-addr s-room"><code>${escapeHtml(roomTxt)}</code>` +
                        `<button class="btn-mini" data-copy="${escapeHtml(roomTxt)}">Копировать комнату</button></div>`;
                    html +=
                        `<div class="s-addr s-copy-all"><span>Скопировать всё</span>` +
                        `<button class="btn-mini" data-copy-all="${escapeHtml(st.relay)}|${escapeHtml(roomTxt)}">Копировать</button></div>`;
                }
                html += '</div>';
            }
            sActAddr = st.relay || '';
        }
        html +=
            `<div class="s-sec">В сети: <b>${st.viewers}</b> зрителей · ` +
            `стримов: ${st.streams.length} · кадров: ${st.frames ?? 0}</div>`;
        sStatusEl.innerHTML = html;
        sStatusEl.classList.remove('hidden');
        sStatusEl.querySelectorAll('[data-copy]').forEach((b) => {
            b.addEventListener('click', () => {
                const t = String(b.dataset.copy);
                const done = () => {
                    b.textContent = 'Готово';
                    setTimeout(() => (b.textContent = 'Скопировать'), 1200);
                };
                if (navigator.clipboard && navigator.clipboard.writeText) {
                    navigator.clipboard.writeText(t).then(done, done);
                } else {
                    const ta = document.createElement('textarea');
                    ta.value = t;
                    document.body.appendChild(ta);
                    ta.select();
                    document.execCommand('copy');
                    ta.remove();
                    done();
                }
            });
        });
        sStatusEl.querySelectorAll('[data-copy-all]').forEach((b) => {
            b.addEventListener('click', () => {
                const [addr, room] = String(b.dataset.copyAll).split('|');
                const t = `${addr}\nкомната: ${room}`;
                const done = () => {
                    b.textContent = 'Готово';
                    setTimeout(() => (b.textContent = 'Копировать'), 1200);
                };
                if (navigator.clipboard && navigator.clipboard.writeText) {
                    navigator.clipboard.writeText(t).then(done, done);
                } else {
                    const ta = document.createElement('textarea');
                    ta.value = t;
                    document.body.appendChild(ta);
                    ta.select();
                    document.execCommand('copy');
                    ta.remove();
                    done();
                }
            });
        });
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
        let relayUrl = sRelayUrl.value.trim();
        if (sMode === 'relay' && relayUrl && !relayUrl.startsWith('ws://') && !relayUrl.startsWith('wss://')) {
            relayUrl = 'ws://' + relayUrl;
            sRelayUrl.value = relayUrl;
        }
        const st =
            sMode === 'local'
                ? await invoke('stream_start', {
                      port: parseInt(sPort.value, 10) || 7335,
                  })
                : await invoke('relay_connect', {
                      url: relayUrl,
                      room: sRoom.value.trim() || null,
                  });
        saveSettings();
        renderSStatus(st);
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
    let url = sRelayUrl.value.trim();
    if (!url) {
        sRelayCheckStatus.textContent = 'введите адрес relay';
        sRelayCheckStatus.classList.remove('ok');
        sRelayCheckStatus.classList.add('err');
        return;
    }
    if (!url.startsWith('ws://') && !url.startsWith('wss://')) {
        url = 'ws://' + url;
        sRelayUrl.value = url;
    }
    sRelayCheck.disabled = true;
    sRelayCheckStatus.textContent = 'проверка…';
    sRelayCheckStatus.classList.remove('ok', 'err');
    try {
        const msg = await invoke('relay_ping', { url });
        sRelayCheckStatus.textContent = '✓ ' + msg;
        sRelayCheckStatus.classList.add('ok');
    } catch (e) {
        sRelayCheckStatus.textContent = '✗ ' + String(e);
        sRelayCheckStatus.classList.add('err');
    } finally {
        sRelayCheck.disabled = false;
        saveSettings();
    }
});

setSeg('local');

const recChatLog = $('chat-log');

const MAX_CHAT_TEXT_LENGTH = 4000;
const MAX_CHAT_IMAGE_BYTES = 512 * 1024;
const MAX_CHAT_IMAGE_BASE64_LENGTH = 4 * Math.ceil(MAX_CHAT_IMAGE_BYTES / 3);

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
        if (msg.markdown === true && window.marked && window.DOMPurify) {
            try {
                const html = window.marked.parse(text, { gfm: true, breaks: true });
                body.classList.add('markdown');
                body.innerHTML = window.DOMPurify.sanitize(html, {
                    ALLOWED_TAGS: ['p', 'br', 'strong', 'em', 'del', 'blockquote', 'ul', 'ol', 'li', 'code', 'pre', 'a', 'h1', 'h2', 'h3', 'h4', 'hr', 'table', 'thead', 'tbody', 'tr', 'th', 'td'],
                    ALLOWED_ATTR: ['href', 'title'],
                });
                body.querySelectorAll('a[href]').forEach((link) => {
                    link.target = '_blank';
                    link.rel = 'noopener noreferrer';
                });
            } catch (_) {
                body.textContent = text;
            }
        } else {
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

listen('stream-chat', (evt) => {
    const m = evt.payload;
    if (typeof m === 'string') {
        try {
            const parsed = JSON.parse(m);
            appendChatMessage(recChatLog, parsed.from ? parsed : { from: '?', text: m });
        } catch (_) {
            appendChatMessage(recChatLog, { from: '?', text: m });
        }
    } else if (m && m.from) {
        appendChatMessage(recChatLog, m);
    }
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
let pendingChatImage = null;
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

async function prepareChatImage(file) {
    const supportedTypes = ['image/jpeg', 'image/png', 'image/webp', 'image/gif', 'image/bmp', 'image/avif'];
    if (!supportedTypes.includes(String(file.type).toLowerCase())) {
        throw new Error('Выберите JPEG, PNG, WebP, GIF, BMP или AVIF-фото.');
    }
    if (file.size > 15 * 1024 * 1024) throw new Error('Исходный файл больше 15 МБ.');

    const bitmap = await createImageBitmap(file);
    try {
        if (bitmap.width * bitmap.height > 30_000_000) {
            throw new Error('Размер фото превышает 30 мегапикселей.');
        }
        const scale = Math.min(1, 1600 / Math.max(bitmap.width, bitmap.height));
        const canvas = document.createElement('canvas');
        canvas.width = Math.max(1, Math.round(bitmap.width * scale));
        canvas.height = Math.max(1, Math.round(bitmap.height * scale));
        canvas.getContext('2d').drawImage(bitmap, 0, 0, canvas.width, canvas.height);

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
                vReconcile(msg.streams);
                if (msg.streams.length === 0) {
                    vSetStatus('подключено, стримов нет — ждите записывающего или проверьте комнату');
                } else {
                    vSetStatus(`подключено (${msg.streams.length} стримов)`);
                }
                if (msg.history) {
                    msg.history.forEach((m) => appendChatMessage(vChatLog, m));
                }
                break;
            case 'streams':
                vReconcile(msg.streams);
                if (msg.streams.length === 0) {
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
                vSetStatus('ошибка: ' + msg.error);
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

function doConnect() {
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
    vConnAborted = false;
    vsock = ws;
    ws.binaryType = 'arraybuffer';
    vSetStatus('подключение…');
    vConnect.disabled = true;
    vDisconnect.disabled = false;

    const connTimer = setTimeout(() => {
        if (ws.readyState === WebSocket.CONNECTING) {
            vConnAborted = true;
            vSetStatus('ошибка: таймаут подключения (проверьте адрес и фаервол сервера)');
            try {
                ws.close();
            } catch (_) {}
            vResetConn();
        }
    }, 8000);

    ws.addEventListener('open', () => {
        clearTimeout(connTimer);
        saveSettings();
        ws.send(
            JSON.stringify({
                type: 'hello',
                role: 'viewer',
                room: vRoom.value.trim() || '',
            })
        );
    });
    ws.addEventListener('message', vHandleMsg);
    ws.addEventListener('close', () => {
        clearTimeout(connTimer);
        if (!vConnAborted) {
            vSetStatus('отключено');
        }
        vResetConn();
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
    if (vsock) {
        try {
            vsock.close();
        } catch (_) {}
    }
    vResetConn();
}

vConnect.addEventListener('click', doConnect);
vDisconnect.addEventListener('click', doDisconnect);

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
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
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
        setChatFeedback('Сообщение отправлено.', 'success');
    } catch (err) {
        setChatFeedback('Не удалось отправить сообщение: ' + String(err), 'error');
    }
});

/* ---------------- boot ---------------- */

loadSettings();
checkPlatformAtStartup();
refresh();
showView('landing');
