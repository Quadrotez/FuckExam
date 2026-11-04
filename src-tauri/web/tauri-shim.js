// Подменяет Tauri IPC внутри браузера: админка на /admin использует те же
// index.html, styles.css и main.js, что и десктоп.
(function () {
    if (window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke) {
        return;
    }

    let socket = null;
    let socketOpen = null;
    let retryTimer = null;
    const listeners = new Map();
    let pending = [];

    function dispatch(event, payload) {
        const handlers = listeners.get(event);
        if (!handlers || !handlers.size) {
            pending.push([event, payload]);
            if (pending.length > 300) pending.shift();
            return;
        }
        handlers.forEach((handler) => {
            try {
                handler({ event, payload });
            } catch (error) {
                console.error('[fuckexam] event handler failed:', error);
            }
        });
    }

    function connectEvents() {
        if (socket && (socket.readyState === WebSocket.OPEN || socket.readyState === WebSocket.CONNECTING)) {
            return;
        }
        const proto = location.protocol === 'https:' ? 'wss' : 'ws';
        let resolveOpen;
        socketOpen = new Promise((resolve) => {
            resolveOpen = resolve;
        });
        const ws = new WebSocket(`${proto}://${location.host}/api/events`);
        socket = ws;

        ws.addEventListener('open', () => {
            if (resolveOpen) resolveOpen();
            resolveOpen = null;
            const queued = pending;
            pending = [];
            queued.forEach(([event, payload]) => dispatch(event, payload));
        });

        ws.addEventListener('message', (message) => {
            if (typeof message.data !== 'string') return;
            let data;
            try {
                data = JSON.parse(message.data);
            } catch (_) {
                return;
            }
            if (!data || typeof data.event !== 'string') return;
            dispatch(data.event, data.payload);
        });

        ws.addEventListener('close', () => {
            if (socket === ws) socket = null;
            socketOpen = null;
            clearTimeout(retryTimer);
            retryTimer = setTimeout(connectEvents, 2000);
        });

        ws.addEventListener('error', () => {});
    }

    async function invoke(cmd, args) {
        const response = await fetch('/api/invoke', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ cmd, args: args || {} }),
        });
        let data = null;
        try {
            data = await response.json();
        } catch (_) {}
        if (!response.ok) {
            throw new Error((data && data.error) || `HTTP ${response.status}`);
        }
        if (data && data.error) throw new Error(data.error);
        return data ? data.result : null;
    }

    function listen(event, handler) {
        connectEvents();
        let handlers = listeners.get(event);
        if (!handlers) {
            handlers = new Set();
            listeners.set(event, handlers);
        }
        handlers.add(handler);
        const open = socketOpen || Promise.resolve();
        const queued = pending.filter(([name]) => name === event);
        pending = pending.filter(([name]) => name !== event);
        queued.forEach(([, payload]) => {
            try {
                handler({ event, payload });
            } catch (error) {
                console.error('[fuckexam] event handler failed:', error);
            }
        });
        return open.then(() => () => handlers.delete(handler));
    }

    window.__TAURI__ = {
        core: { invoke },
        event: { listen },
        window: {
            getCurrentWindow: () => ({ label: 'main' }),
        },
    };
})();
