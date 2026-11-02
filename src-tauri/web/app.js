const statusEl = document.getElementById('status');
const statusBar = document.querySelector('.bar');
const gridEl = document.getElementById('grid');
const chatLog = document.getElementById('chat-log');
const chatForm = document.getElementById('chat-form');
const chatInput = document.getElementById('chat-input');
const chatFeedback = document.getElementById('chat-feedback');
const nameInput = document.getElementById('name');
const markdownBox = document.getElementById('markdown');
const adminLink = document.getElementById('admin-link');

const MAX_TEXT = 4000;
const streams = new Map();
let socket = null;
let userClosed = false;
let reconnectDelay = 1000;
let reconnectTimer = null;

try {
    nameInput.value = localStorage.getItem('fex-name') || '';
} catch (_) {}

function setStatus(text, live) {
    statusEl.textContent = text;
    statusBar.classList.toggle('live', !!live);
}

function feedback(text, kind = '') {
    chatFeedback.textContent = text;
    chatFeedback.classList.toggle('error', kind === 'error');
    chatFeedback.classList.toggle('success', kind === 'success');
}

function ensureCell(node, w, h) {
    let cell = streams.get(node);
    if (!cell) {
        const wrap = document.createElement('div');
        wrap.className = 'cell';
        wrap.title = 'клик — полный экран';
        const label = document.createElement('span');
        label.className = 'node';
        label.textContent = `node ${node}`;
        const canvas = document.createElement('canvas');
        canvas.width = 640;
        canvas.height = 360;
        wrap.append(label, canvas);
        wrap.addEventListener('click', () => {
            if (document.fullscreenElement === wrap) document.exitFullscreen().catch(() => {});
            else wrap.requestFullscreen().catch(() => {});
        });
        gridEl.appendChild(wrap);
        cell = { wrap, canvas, rendering: false, pending: null };
        streams.set(node, cell);
    }
    if (w && h) {
        const cw = Math.min(w, 1920);
        const ch = Math.max(1, Math.round((h / w) * cw));
        if (cell.canvas.width !== cw || cell.canvas.height !== ch) {
            cell.canvas.width = cw;
            cell.canvas.height = ch;
        }
        cell.wrap.style.aspectRatio = `${w} / ${h}`;
    }
    return cell;
}

function removeCell(node) {
    const cell = streams.get(node);
    if (cell) {
        cell.wrap.remove();
        streams.delete(node);
    }
}

function reconcile(list) {
    const ids = new Set(list.map((item) => item.id));
    for (const node of [...streams.keys()]) {
        if (!ids.has(node)) removeCell(node);
    }
    list.forEach((item) => ensureCell(item.id, item.w, item.h));
}

function drawFrame(cell, jpeg) {
    if (cell.rendering) {
        cell.pending = jpeg;
        return;
    }
    cell.rendering = true;
    const img = new Image();
    const url = URL.createObjectURL(new Blob([jpeg], { type: 'image/jpeg' }));
    img.onload = () => {
        try {
            cell.canvas.getContext('2d').drawImage(img, 0, 0, cell.canvas.width, cell.canvas.height);
        } catch (_) {}
        URL.revokeObjectURL(url);
        cell.rendering = false;
        const next = cell.pending;
        cell.pending = null;
        if (next) drawFrame(cell, next);
    };
    img.onerror = () => {
        URL.revokeObjectURL(url);
        cell.rendering = false;
        cell.pending = null;
    };
    img.src = url;
}

function appendChat(msg) {
    if (!msg || (!msg.text && !msg.image)) return;
    const div = document.createElement('div');
    div.className = 'msg';
    const from = document.createElement('span');
    from.className = 'from';
    from.textContent = String(msg.from || 'зритель');
    div.appendChild(from);

    if (msg.text) {
        const body = document.createElement('span');
        if (msg.markdown === true && window.marked && window.DOMPurify) {
            try {
                const html = window.marked.parse(String(msg.text), { gfm: true, breaks: true });
                body.innerHTML = window.DOMPurify.sanitize(html, {
                    ALLOWED_TAGS: ['p', 'br', 'strong', 'em', 'del', 'blockquote', 'ul', 'ol', 'li', 'code', 'pre', 'a', 'h1', 'h2', 'h3', 'h4', 'hr', 'table', 'thead', 'tbody', 'tr', 'th', 'td'],
                    ALLOWED_ATTR: ['href', 'title'],
                });
                body.querySelectorAll('a[href]').forEach((link) => {
                    link.target = '_blank';
                    link.rel = 'noopener noreferrer';
                });
            } catch (_) {
                body.textContent = String(msg.text);
            }
        } else {
            body.textContent = String(msg.text);
        }
        div.appendChild(body);
    }

    if (msg.image && typeof msg.image.data === 'string' && msg.image.data.length < 700000) {
        const img = document.createElement('img');
        img.loading = 'lazy';
        img.alt = String(msg.image.name || 'фото');
        img.src = `data:image/jpeg;base64,${msg.image.data}`;
        div.appendChild(img);
    }

    chatLog.appendChild(div);
    chatLog.scrollTop = chatLog.scrollHeight;
    while (chatLog.children.length > 100) chatLog.removeChild(chatLog.firstChild);
}

function handleText(text) {
    let msg;
    try {
        msg = JSON.parse(text);
    } catch (_) {
        return;
    }
    switch (msg.type) {
        case 'welcome':
            chatLog.innerHTML = '';
            reconcile(msg.streams || []);
            (msg.history || []).forEach(appendChat);
            if (!msg.streams || !msg.streams.length) {
                setStatus('подключено, трансляция ещё не запущена', false);
            } else {
                setStatus(`подключено (${msg.streams.length} стримов)`, true);
            }
            break;
        case 'streams':
            reconcile(msg.streams || []);
            setStatus(
                msg.streams && msg.streams.length
                    ? `в эфире: ${msg.streams.length} стримов`
                    : 'подключено, трансляция ещё не запущена',
                !!(msg.streams && msg.streams.length)
            );
            break;
        case 'stream-end':
            removeCell(msg.id);
            break;
        case 'chat':
            appendChat(msg);
            break;
        case 'error':
            setStatus('ошибка: ' + (msg.error || 'неизвестно'), false);
            break;
    }
}

function handleBinary(buffer) {
    if (buffer.byteLength < 5) return;
    const view = new DataView(buffer);
    const node = view.getUint32(0, true);
    const cell = ensureCell(node);
    drawFrame(cell, buffer.slice(4));
}

function connect() {
    clearTimeout(reconnectTimer);
    if (userClosed) return;
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    const url = `${proto}://${location.host}/ws`;
    let ws;
    try {
        ws = new WebSocket(url);
    } catch (e) {
        setStatus('не удалось открыть соединение: ' + e);
        scheduleReconnect();
        return;
    }
    socket = ws;
    ws.binaryType = 'arraybuffer';
    setStatus('подключение…', false);

    ws.addEventListener('open', () => {
        reconnectDelay = 1000;
        ws.send(JSON.stringify({ type: 'hello', role: 'viewer' }));
    });

    ws.addEventListener('message', (event) => {
        if (typeof event.data === 'string') handleText(event.data);
        else if (event.data instanceof ArrayBuffer) handleBinary(event.data);
        else if (event.data instanceof Blob) {
            event.data.arrayBuffer().then(handleBinary).catch(() => {});
        }
    });

    ws.addEventListener('close', () => {
        if (socket !== ws) return;
        socket = null;
        if (userClosed) return;
        setStatus('соединение потеряно, переподключаемся…', false);
        scheduleReconnect();
    });

    ws.addEventListener('error', () => {
        setStatus('ошибка соединения', false);
    });
}

function scheduleReconnect() {
    if (userClosed) return;
    clearTimeout(reconnectTimer);
    const delay = reconnectDelay;
    reconnectDelay = Math.min(reconnectDelay * 2, 15000);
    reconnectTimer = setTimeout(() => {
        if (!document.hidden) connect();
    }, delay);
}

document.addEventListener('visibilitychange', () => {
    if (!document.hidden && !socket && !userClosed) {
        reconnectDelay = 1000;
        connect();
    }
});
window.addEventListener('online', () => {
    if (!socket && !userClosed) connect();
});

chatInput.addEventListener('keydown', (event) => {
    if (event.key === 'Enter' && !event.ctrlKey && !event.shiftKey && !event.metaKey && !event.isComposing) {
        event.preventDefault();
        chatForm.requestSubmit();
    }
});

chatForm.addEventListener('submit', (event) => {
    event.preventDefault();
    const text = chatInput.value.trim();
    if (!text) return;
    if (text.length > MAX_TEXT) {
        feedback(`Сообщение слишком длинное (максимум ${MAX_TEXT}).`, 'error');
        return;
    }
    if (!socket || socket.readyState !== WebSocket.OPEN) {
        feedback('Нет соединения с трансляцией.', 'error');
        return;
    }
    const name = nameInput.value.trim() || 'зритель';
    try {
        localStorage.setItem('fex-name', name);
    } catch (_) {}
    socket.send(JSON.stringify({
        type: 'chat',
        from: name,
        text,
        markdown: markdownBox.checked,
    }));
    chatInput.value = '';
    feedback('Сообщение отправлено.', 'success');
});

fetch('/api/status')
    .then((response) => (response.ok ? response.json() : null))
    .then((data) => {
        if (data && adminLink) adminLink.hidden = false;
    })
    .catch(() => {});

connect();
