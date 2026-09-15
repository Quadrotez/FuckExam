const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);

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
const sStatusBox = document.querySelector('#view-recorder .status-bar');

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
        const st =
            sMode === 'local'
                ? await invoke('stream_start', {
                      port: parseInt(sPort.value, 10) || 7335,
                  })
                : await invoke('relay_connect', {
                      url: sRelayUrl.value.trim(),
                      room: sRoom.value.trim() || null,
                  });
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
setSeg('local');

const recChatLog = $('chat-log');

function recChatAppend(msg) {
    const div = document.createElement('div');
    div.className = 'msg';
    div.innerHTML = `<span class="from">${escapeHtml(msg.from)}</span><span class="text">${escapeHtml(msg.text)}</span>`;
    recChatLog.appendChild(div);
    recChatLog.scrollTop = recChatLog.scrollHeight;
    while (recChatLog.children.length > 100) {
        recChatLog.removeChild(recChatLog.firstChild);
    }
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
            recChatAppend(JSON.parse(m).from ? JSON.parse(m) : { from: '?', text: m });
        } catch (_) {
            recChatAppend({ from: '?', text: m });
        }
    } else if (m && m.from) {
        recChatAppend(m);
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

let vsock = null;
let vFrames = 0;
const vstreams = new Map(); // node -> { img, stale }

function vSetStatus(text) {
    vStatusText.textContent = text;
}

function vChatAppend(html) {
    const div = document.createElement('div');
    div.className = 'msg';
    div.innerHTML = html;
    vChatLog.appendChild(div);
    vChatLog.scrollTop = vChatLog.scrollHeight;
    while (vChatLog.children.length > 100) {
        vChatLog.removeChild(vChatLog.firstChild);
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
                vSetStatus(`подключено (${msg.streams.length} стримов)`);
                vFrames = 0;
                vReconcile(msg.streams);
                if (msg.history) {
                    msg.history.forEach((m) =>
                        vChatAppend(
                            `<span class="from">${escapeHtml(m.from)}</span><span class="text">${escapeHtml(m.text)}</span>`
                        )
                    );
                }
                break;
            case 'streams':
                vReconcile(msg.streams);
                vSetStatus(`подключено (${msg.streams.length} стримов)`);
                break;
            case 'stream-end':
                vRemoveImg(msg.id);
                break;
            case 'chat':
                if (msg.from && msg.text) {
                    vChatAppend(
                        `<span class="from">${escapeHtml(msg.from)}</span><span class="text">${escapeHtml(msg.text)}</span>`
                    );
                }
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
    const url = vUrl.value.trim();
    if (!url) {
        vSetStatus('введите адрес ws://…');
        return;
    }
    let ws;
    try {
        ws = new WebSocket(url);
    } catch (e) {
        vSetStatus('неверный адрес');
        return;
    }
    vsock = ws;
    ws.binaryType = 'arraybuffer';
    vSetStatus('подключение…');
    vConnect.disabled = true;
    vDisconnect.disabled = false;

    ws.addEventListener('open', () => {
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
        vSetStatus('отключено');
        vResetConn();
    });
    ws.addEventListener('error', () => {
        vSetStatus('ошибка соединения');
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

vChatForm.addEventListener('submit', (e) => {
    e.preventDefault();
    const text = vChatInput.value.trim();
    vChatInput.value = '';
    if (!text || !vsock || vsock.readyState !== WebSocket.OPEN) return;
    vsock.send(JSON.stringify({ type: 'chat', from: vName.value.trim() || 'зритель', text }));
});

/* ---------------- boot ---------------- */

refresh();
showView('landing');