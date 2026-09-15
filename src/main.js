const { invoke } = window.__TAURI__.core;

const btn       = document.getElementById('btn');
const statusTxt = document.getElementById('status-text');
const statusBar = document.querySelector('.status-bar');
const timerEl   = document.getElementById('timer');
const windowsEl = document.getElementById('windows');
const pathHint  = document.getElementById('path-hint');

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
    if (tickHandle) { clearInterval(tickHandle); tickHandle = null; }
    timerEl.classList.add('hidden');
    timerEl.textContent = '';
}

function renderWindows(list) {
    if (list && list.length) {
        lastFiles = list.map(w => w.output);
        windowsEl.innerHTML = '';
        list.forEach((w, i) => {
            const div = document.createElement('div');
            div.className = 'win-item';
            div.style.animationDelay = `${i * 70}ms`;
            const res =
                w.width > 0 && w.height > 0 ? `<span class="res">${w.width}×${w.height}</span>` : '';
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
            const msg = await invoke('start_recording');
            renderWindows((await invoke('recording_status')).windows);
            await refresh();
        }
    } catch (e) {
        const err = String(e);
        setStatus('ошибка: ' + err, false);
        await refresh();
    }
}

btn.addEventListener('click', handleClick);
refresh();