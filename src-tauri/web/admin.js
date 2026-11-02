const $ = (id) => document.getElementById(id);

const fields = ['port', 'fps', 'quality', 'maxWidth', 'relayUrl', 'mode', 'webOnLaunch', 'closeToTray'];

function feedback(id, text, kind = '') {
    const el = $(id);
    el.textContent = text;
    el.classList.toggle('error', kind === 'error');
    el.classList.toggle('success', kind === 'success');
}

async function api(method, path, body) {
    const response = await fetch(path, {
        method,
        headers: body ? { 'Content-Type': 'application/json' } : undefined,
        body: body ? JSON.stringify(body) : undefined,
    });
    const data = await response.json().catch(() => null);
    if (!response.ok) {
        throw new Error((data && data.error) || `HTTP ${response.status}`);
    }
    return data;
}

function fillSettings(settings) {
    if (!settings) return;
    $('port').value = settings.port;
    $('fps').value = settings.fps;
    $('quality').value = settings.quality;
    $('maxWidth').value = settings.maxWidth;
    $('relayUrl').value = settings.relayUrl || '';
    $('mode').value = settings.mode || 'local';
    $('webOnLaunch').checked = settings.webOnLaunch !== false;
    $('closeToTray').checked = settings.closeToTray !== false;
}

function collectSettings() {
    return {
        port: parseInt($('port').value, 10) || 7335,
        fps: parseInt($('fps').value, 10) || 24,
        quality: parseInt($('quality').value, 10) || 75,
        maxWidth: parseInt($('maxWidth').value, 10) || 1280,
        relayUrl: $('relayUrl').value.trim(),
        mode: $('mode').value,
        webOnLaunch: $('webOnLaunch').checked,
        closeToTray: $('closeToTray').checked,
    };
}

function renderStatus(payload) {
    if (!payload) return;
    const stream = payload.stream || {};
    $('st-active').textContent = stream.active ? 'идёт' : 'остановлена';
    $('st-mode').textContent = stream.active
        ? stream.mode === 'relay'
            ? 'через relay'
            : 'локально'
        : '—';
    $('st-web').textContent = stream.web && stream.web.length ? stream.web.join(', ') : '—';
    $('st-relay').textContent = stream.relay
        ? `${stream.relay}${stream.room ? ' · ' + stream.room : ''}`
        : '—';
    $('st-viewers').textContent = `${stream.viewers ?? 0} / ${stream.frames ?? 0}`;
    $('st-port').textContent = payload.webPort ?? '—';
    $('btn-start').disabled = !!stream.active;
    $('btn-stop').disabled = !stream.active;
    fillSettings(payload.settings);
}

async function refreshStatus() {
    try {
        renderStatus(await api('GET', '/api/status'));
    } catch (e) {
        $('st-active').textContent = 'нет связи с приложением';
    }
}

async function act(path, message) {
    try {
        feedback('status-feedback', message);
        renderStatus(await api('POST', path));
        feedback('status-feedback', message, 'success');
    } catch (e) {
        feedback('status-feedback', String(e), 'error');
    }
}

$('btn-start').addEventListener('click', () => act('/api/stream/start', 'трансляция запускается…'));
$('btn-stop').addEventListener('click', () => act('/api/stream/stop', 'трансляция остановлена'));
$('btn-tray').addEventListener('click', () => act('/api/window/tray', 'окно скрыто в трей'));
$('btn-show').addEventListener('click', () => act('/api/window/show', 'окно показано'));
$('btn-hide').addEventListener('click', () => act('/api/window/hide', 'приложение скрыто полностью'));
$('btn-quit').addEventListener('click', async () => {
    if (!confirm('Закрыть FuckExam?')) return;
    try {
        await api('POST', '/api/quit');
        feedback('status-feedback', 'приложение закрывается', 'success');
    } catch (e) {
        feedback('status-feedback', String(e), 'error');
    }
});

$('settings-form').addEventListener('submit', async (event) => {
    event.preventDefault();
    try {
        const saved = await api('POST', '/api/settings', collectSettings());
        fillSettings(saved);
        feedback('settings-feedback', 'Сохранено.', 'success');
        setTimeout(refreshStatus, 400);
    } catch (e) {
        feedback('settings-feedback', String(e), 'error');
    }
});

$('btn-reload').addEventListener('click', async () => {
    try {
        fillSettings(await api('GET', '/api/settings'));
        feedback('settings-feedback', 'Значения восстановлены из сохранённых.', '');
    } catch (e) {
        feedback('settings-feedback', String(e), 'error');
    }
});

refreshStatus();
setInterval(() => {
    if (!document.hidden) refreshStatus();
}, 2000);

fields.forEach((field) => {
    $(field).addEventListener('change', () => feedback('settings-feedback', ''));
});
