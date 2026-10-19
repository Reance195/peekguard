// PeekGuard uTools 插件的前端逻辑。
//
// 面板（feature: panel）显示连接状态、绑定信息与全部操作；
// 快捷指令（quick-toggle / quick-bind）隐藏窗口直接执行命令，用系统通知反馈结果。

const ut =
  typeof utools !== 'undefined'
    ? utools
    : {
        // 纯浏览器预览时的空实现，方便调样式
        onPluginEnter: (fn) => fn({ code: 'panel' }),
        onPluginOut: () => {},
        hideMainWindow: () => {},
        outPlugin: () => {},
        showNotification: (t) => console.log('[notification]', t),
        showOpenDialog: () => null,
      };

const pg = window.peekguard || {
  send: async () => ({ ok: false, offline: true, err: 'preload 未加载' }),
  probe: () => ({ found: false, path: null, source: '', candidates: [] }),
  launch: () => ({ ok: false, err: 'preload 未加载' }),
  waitReady: async () => false,
  getExePath: () => '',
  setExePath: () => false,
  clearExePath: () => false,
};

const STATE_LABEL = {
  idle: '未绑定',
  armed: '守护中',
  pending: '即将隐身',
  hidden: '已隐身',
  latched: '已隐身 · 锁定',
};

const $ = (id) => document.getElementById(id);

let busy = false;
let lastState = null;
let pollTimer = null;
let toastTimer = null;

// ---------------------------------------------------------------- 事件入口

ut.onPluginEnter(async ({ code }) => {
  if (code === 'quick-toggle') {
    await runQuick('toggle');
    return;
  }
  if (code === 'quick-bind') {
    await runQuick('bind');
    return;
  }
  startPolling();
  await refresh();
});

ut.onPluginOut(() => stopPolling());

/** 快捷指令：不留窗口，直接执行一条命令，结果用系统通知反馈。 */
async function runQuick(command) {
  ut.hideMainWindow();
  let r = await pg.send(command);
  if (r.offline) {
    const found = pg.probe();
    if (!found.found) {
      ut.showNotification('未找到 PeekGuard，请打开控制面板手动指定');
    } else {
      ut.showNotification('正在启动 PeekGuard…');
      const l = pg.launch(found.path);
      if (!l.ok) {
        ut.showNotification(l.err);
      } else if (!(await pg.waitReady(8000))) {
        ut.showNotification('PeekGuard 启动超时，请手动打开检查');
      } else {
        r = await pg.send(command);
        ut.showNotification(r.ok ? r.msg || '完成' : r.err || '操作失败');
      }
    }
  } else {
    ut.showNotification(r.ok ? r.msg || '完成' : r.err || '操作失败');
  }
  ut.outPlugin(true);
}

// ---------------------------------------------------------------- 面板数据流

async function refresh() {
  if (busy) return;
  const r = await pg.send('status');
  if (busy) return;
  if (r.offline) {
    lastState = null;
    renderOffline(true);
    renderPanel(null);
    return;
  }
  lastState = r.state || null;
  renderOffline(false);
  renderPanel(lastState);
}

/** 执行一条面板命令，并用响应里带回的状态直接刷新界面。 */
async function act(command) {
  if (busy) return;
  busy = true;
  setControlsDisabled(true);
  try {
    const r = await pg.send(command);
    if (r.offline) {
      lastState = null;
      renderOffline(true);
      renderPanel(null);
    } else {
      lastState = r.state || null;
      renderOffline(false);
      renderPanel(lastState);
      if (!r.ok) flash(r.err || '操作失败');
    }
  } finally {
    busy = false;
    setControlsDisabled(false);
    if (lastState) renderPanel(lastState);
  }
}

// ---------------------------------------------------------------- 渲染

function renderPanel(state) {
  const panel = $('panel');
  if (!state) {
    panel.classList.add('hidden');
    return;
  }
  panel.classList.remove('hidden');

  $('conn').textContent = '已连接 · v' + (state.version || '');
  $('conn').className = 'conn ok';

  const label = STATE_LABEL[state.state] || state.state || '未知';
  $('badge').textContent = label;
  $('badge').dataset.state = state.state || '';

  $('state-label').textContent = label;
  $('target').textContent = state.bound && state.bound_desc ? state.bound_desc : '没有绑定任何窗口';
  $('admin-chip').classList.toggle('hidden', !state.elevated);

  const hiddenNow = state.state === 'hidden' || state.state === 'latched';
  const toggle = $('btn-toggle');
  toggle.textContent = hiddenNow ? '恢复显示' : '立即隐身';
  toggle.classList.toggle('restore', hiddenNow);

  $('btn-unbind').disabled = !state.bound;
  $('sw-latch').checked = !!state.latch;
  $('sw-scan').checked = !!state.scan;
  $('sw-autostart').checked = !!state.autostart;

  for (const b of document.querySelectorAll('#mode button')) {
    b.classList.toggle('on', b.dataset.mode === state.mode);
  }

  $('meta').textContent = state.config || '';
  $('meta').title = state.config || '';
}

function renderOffline(show) {
  const box = $('offline');
  if (!show) {
    box.classList.add('hidden');
    return;
  }
  box.classList.remove('hidden');
  $('conn').textContent = '未连接 · PeekGuard 未运行';
  $('conn').className = 'conn off';
  $('badge').textContent = '未运行';
  $('badge').dataset.state = 'offline';

  const found = pg.probe();
  const pathEl = $('exe-path');
  if (found.found) {
    pathEl.textContent = '已找到 ' + found.path + '（' + found.source + '）';
    $('btn-launch').disabled = false;
  } else {
    pathEl.textContent = '没有找到 peekguard.exe。手动指定一次后会记住。';
    $('btn-launch').disabled = true;
  }
}

function setControlsDisabled(disabled) {
  for (const el of document.querySelectorAll('#panel button, #panel input')) el.disabled = disabled;
}

function flash(text) {
  const el = $('toast');
  el.textContent = text;
  el.classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.remove('show'), 2800);
}

function startPolling() {
  stopPolling();
  pollTimer = setInterval(refresh, 2000);
}

function stopPolling() {
  if (pollTimer) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
}

// ---------------------------------------------------------------- 交互绑定

function bindUi() {
  $('btn-toggle').addEventListener('click', () => act('toggle'));
  $('btn-bind').addEventListener('click', () => act('bind'));
  $('btn-unbind').addEventListener('click', () => act('unbind'));
  $('btn-reload').addEventListener('click', () => act('reload'));
  $('sw-latch').addEventListener('change', (e) => act('latch ' + (e.target.checked ? 'on' : 'off')));
  $('sw-scan').addEventListener('change', (e) => act('scan ' + (e.target.checked ? 'on' : 'off')));
  $('sw-autostart').addEventListener('change', (e) =>
    act('autostart ' + (e.target.checked ? 'on' : 'off'))
  );
  for (const b of document.querySelectorAll('#mode button')) {
    b.addEventListener('click', () => act('mode ' + b.dataset.mode));
  }

  $('btn-launch').addEventListener('click', onLaunchClicked);
  $('btn-choose').addEventListener('click', onChooseClicked);
  $('btn-retry').addEventListener('click', () => refresh());
  $('btn-out').addEventListener('click', () => ut.outPlugin());
}

async function onLaunchClicked() {
  const found = pg.probe();
  if (!found.found) return;
  const btn = $('btn-launch');
  btn.disabled = true;
  btn.textContent = '正在启动…';
  const l = pg.launch(found.path);
  const ok = l.ok ? await pg.waitReady(8000) : false;
  btn.textContent = '启动 PeekGuard';
  if (ok) {
    flash('PeekGuard 已启动');
    await refresh();
  } else {
    btn.disabled = false;
    flash(l.ok ? '启动超时，请手动打开 PeekGuard 检查' : l.err);
  }
}

function onChooseClicked() {
  const files = ut.showOpenDialog({
    title: '选择 peekguard.exe',
    filters: [{ name: 'PeekGuard', extensions: ['exe'] }],
    properties: ['openFile'],
  });
  if (!files || !files.length) return;
  if (!pg.setExePath(files[0])) {
    flash('无法保存路径（存储不可用）');
    return;
  }
  flash('已记住：' + files[0]);
  refresh();
}

document.addEventListener('DOMContentLoaded', bindUi);
