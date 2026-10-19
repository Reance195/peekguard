// PeekGuard uTools 插件的桥接层（preload）。
//
// 运行在 uTools 的 Node.js preload 环境，只向页面暴露最小接口：
//
//   window.peekguard.send(command)      给 PeekGuard 发一条命令，返回响应对象
//   window.peekguard.probe()            探测 peekguard.exe 的常见位置
//   window.peekguard.launch(exePath)    以分离进程启动 PeekGuard
//   window.peekguard.waitReady(ms)      轮询等待管道就绪
//   window.peekguard.getExePath()       读取手动指定的 exe 路径
//   window.peekguard.setExePath(path)   记住手动指定的 exe 路径
//   window.peekguard.clearExePath()     清除手动指定
//
// 与 PeekGuard 的通信走本机命名管道（默认 \\.\pipe\PeekGuard-IPC-v1），
// 不监听网络端口；管道不可达即视为 PeekGuard 未运行（离线）。

const net = require('net');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { spawn } = require('child_process');

const DEFAULT_PIPE = '\\\\.\\pipe\\PeekGuard-IPC-v1';
const REQUEST_TIMEOUT_MS = 5000;
const EXE_STORE_KEY = 'peekguard.exePath';

/** 管道名：默认与 PeekGuard 的 ipc.rs 保持一致，可用 PEEKGUARD_PIPE 覆盖（测试隔离）。 */
function pipePath() {
  const raw = process.env.PEEKGUARD_PIPE;
  if (!raw || !raw.trim()) return DEFAULT_PIPE;
  const name = raw.trim();
  return name.startsWith('\\\\.\\pipe\\') ? name : '\\\\.\\pipe\\' + name;
}

/** 给 PeekGuard 发一条命令（一行文本），返回解析后的响应对象。 */
function send(command) {
  return new Promise((resolve) => {
    let settled = false;
    let buffer = '';
    const socket = net.createConnection(pipePath());
    const finish = (value) => {
      if (settled) return;
      settled = true;
      socket.destroy();
      resolve(value);
    };
    socket.setTimeout(REQUEST_TIMEOUT_MS);
    socket.on('connect', () => socket.write(command + '\n'));
    socket.on('data', (chunk) => {
      buffer += chunk.toString('utf8');
      const nl = buffer.indexOf('\n');
      if (nl < 0) return;
      const line = buffer.slice(0, nl);
      try {
        finish(JSON.parse(line));
      } catch (_) {
        finish({ ok: false, err: '无法解析 PeekGuard 的响应：' + line });
      }
    });
    socket.on('error', () => finish({ ok: false, offline: true, err: 'PeekGuard 未运行' }));
    socket.on('timeout', () => finish({ ok: false, err: 'PeekGuard 响应超时' }));
    socket.on('close', () => finish({ ok: false, offline: true, err: 'PeekGuard 未运行' }));
  });
}

/** uTools 数据存储读取；在纯 Node 环境下退化为 null。 */
function storeGet(key) {
  try {
    if (typeof utools !== 'undefined' && utools.dbStorage) return utools.dbStorage.getItem(key);
  } catch (_) {}
  return null;
}

/** uTools 数据存储写入；value 为 null 时删除。 */
function storeSet(key, value) {
  try {
    if (typeof utools !== 'undefined' && utools.dbStorage) {
      if (value == null) utools.dbStorage.removeItem(key);
      else utools.dbStorage.setItem(key, value);
      return true;
    }
  } catch (_) {}
  return false;
}

function getExePath() {
  const v = storeGet(EXE_STORE_KEY);
  return typeof v === 'string' && v.trim() ? v.trim() : '';
}

function setExePath(p) {
  return storeSet(EXE_STORE_KEY, p ? String(p) : null);
}

function clearExePath() {
  return storeSet(EXE_STORE_KEY, null);
}

/**
 * 探测 peekguard.exe：手动指定的路径 > 环境变量 > 常见安装目录 > 插件目录 > PATH。
 * 返回 { found, path, source, candidates }，candidates 是去重后的候选位置（供排障展示）。
 */
function probe() {
  const seen = new Set();
  const candidates = [];
  const push = (p, source) => {
    if (!p) return;
    let normalized;
    try {
      normalized = path.resolve(p);
    } catch (_) {
      return;
    }
    const key = normalized.toLowerCase();
    if (seen.has(key)) return;
    seen.add(key);
    candidates.push({ path: normalized, source });
  };

  push(getExePath(), '手动指定');
  push(process.env.PEEKGUARD_EXE, 'PEEKGUARD_EXE 环境变量');
  const home = os.homedir();
  const local = process.env.LOCALAPPDATA || path.join(home, 'AppData', 'Local');
  push(path.join(local, 'Programs', 'peekguard', 'peekguard.exe'), '默认安装目录');
  push(path.join(home, 'Downloads', 'peekguard.exe'), '下载目录');
  push(path.join(home, 'Desktop', 'peekguard.exe'), '桌面');
  push(path.join(__dirname, 'peekguard.exe'), '插件目录');
  for (const dir of (process.env.PATH || '').split(';')) {
    if (dir.trim()) push(path.join(dir.trim(), 'peekguard.exe'), 'PATH');
  }

  for (const c of candidates) {
    try {
      if (fs.existsSync(c.path)) return { found: true, path: c.path, source: c.source, candidates };
    } catch (_) {}
  }
  return { found: false, path: null, source: '', candidates };
}

/** 以分离进程启动 PeekGuard（关闭插件/终端后它继续驻留托盘）。 */
function launch(exePath) {
  const target = exePath || probe().path || '';
  if (!target) return { ok: false, err: '没有找到 peekguard.exe，请在面板中手动指定' };
  try {
    if (!fs.existsSync(target)) return { ok: false, err: '文件不存在：' + target };
    const child = spawn(target, [], { detached: true, stdio: 'ignore', windowsHide: true });
    child.unref();
    return { ok: true, path: target };
  } catch (e) {
    return { ok: false, err: '启动失败：' + (e && e.message ? e.message : e) };
  }
}

/** 轮询等待 PeekGuard 的管道就绪（启动后通常几百毫秒内可达）。 */
async function waitReady(timeoutMs = 6000) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const r = await send('ping');
    if (!r.offline) return true;
    if (Date.now() >= deadline) return false;
    await new Promise((res) => setTimeout(res, 250));
  }
}

window.peekguard = {
  send,
  probe,
  launch,
  waitReady,
  getExePath,
  setExePath,
  clearExePath,
};
