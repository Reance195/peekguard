#!/usr/bin/env node
// 命令行测试工具：不依赖 uTools，直接通过命名管道把 PeekGuard 的 IPC 命令打一遍。
//
//   node test-cli.js                跑内置全套用例（只做可复原的操作）
//   node test-cli.js status         发一条命令，打印原始 JSON 响应
//   node test-cli.js latch on       带参数的命令照写
//
// 环境变量 PEEKGUARD_PIPE 可指向隔离的测试实例（与 PeekGuard 的 ipc.rs 一致）。
// 注意：hide/show/toggle/bind 等命令会真实作用于正在运行的 PeekGuard。

const net = require('net');

function pipePath() {
  const raw = process.env.PEEKGUARD_PIPE;
  if (!raw || !raw.trim()) return '\\\\.\\pipe\\PeekGuard-IPC-v1';
  const name = raw.trim();
  return name.startsWith('\\\\.\\pipe\\') ? name : '\\\\.\\pipe\\' + name;
}

function send(command, timeoutMs = 5000) {
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
    socket.setTimeout(timeoutMs);
    socket.on('connect', () => socket.write(command + '\n'));
    socket.on('data', (chunk) => {
      buffer += chunk.toString('utf8');
      const nl = buffer.indexOf('\n');
      if (nl < 0) return;
      const line = buffer.slice(0, nl);
      try {
        finish(JSON.parse(line));
      } catch (_) {
        finish({ ok: false, err: '无法解析响应：' + line });
      }
    });
    socket.on('error', () =>
      finish({ ok: false, offline: true, err: 'PeekGuard 未运行（管道不存在）' })
    );
    socket.on('timeout', () => finish({ ok: false, err: '响应超时' }));
    socket.on('close', () => finish({ ok: false, offline: true, err: '连接已关闭' }));
  });
}

async function fullSuite() {
  let failures = 0;
  const check = (name, ok, detail) => {
    if (!ok) failures += 1;
    const mark = ok ? 'PASS' : 'FAIL';
    const extra = !ok && detail ? '  → ' + detail : '';
    console.log(`${mark}  ${name}${extra}`);
  };

  const st0 = await send('status');
  if (st0.offline) {
    console.error('无法连接 PeekGuard：' + st0.err);
    console.error('请先运行 peekguard.exe，或用 PEEKGUARD_PIPE 指向测试实例。');
    process.exit(2);
  }

  check('status 返回 ok', st0.ok === true, st0.err);
  const s = st0.state || {};
  check('status 带版本号', typeof s.version === 'string' && s.version.length > 0);
  check(
    'status 状态字段齐全',
    ['bound', 'state', 'mode', 'latch', 'scan', 'rules', 'autostart', 'config'].every(
      (k) => k in s
    ),
    JSON.stringify(s)
  );

  const ping = await send('ping');
  check('ping 返回 ok', ping.ok === true, ping.err);

  if (!s.bound) {
    const h = await send('hide');
    check('未绑定时 hide 给出绑定提示', h.ok === false && /绑定/.test(h.err || ''), JSON.stringify(h));
    const sh = await send('show');
    check('未绑定时 show 给出绑定提示', sh.ok === false && /绑定/.test(sh.err || ''), JSON.stringify(sh));
    const t = await send('toggle');
    check('未绑定时 toggle 给出绑定提示', t.ok === false && /绑定/.test(t.err || ''), JSON.stringify(t));
    const u = await send('unbind');
    check('未绑定时 unbind 提示无需操作', u.ok === true, JSON.stringify(u));
  } else {
    console.log('SKIP  已绑定窗口，跳过 hide/show/toggle（避免打扰当前正在使用的窗口）');
  }

  const l1 = await send('latch ' + (s.latch ? 'off' : 'on'));
  check('latch 翻转生效', l1.ok === true && l1.state && l1.state.latch === !s.latch, JSON.stringify(l1));
  const l2 = await send('latch ' + (s.latch ? 'on' : 'off'));
  check('latch 复原', l2.ok === true && l2.state && l2.state.latch === s.latch, JSON.stringify(l2));
  const l3 = await send('latch maybe');
  check('latch 非法参数报错', l3.ok === false, JSON.stringify(l3));

  const other = s.mode === 'alpha' ? 'move' : 'alpha';
  const m1 = await send('mode ' + other);
  check('mode 切换生效', m1.ok === true && m1.state && m1.state.mode === other, JSON.stringify(m1));
  const mBad = await send('mode not-a-mode');
  check('mode 非法参数报错', mBad.ok === false, JSON.stringify(mBad));
  const m2 = await send('mode ' + s.mode);
  check('mode 复原', m2.ok === true && m2.state && m2.state.mode === s.mode, JSON.stringify(m2));

  const s1 = await send('scan ' + (s.scan ? 'off' : 'on'));
  check('scan 翻转生效', s1.ok === true && s1.state && s1.state.scan === !s.scan, JSON.stringify(s1));
  const s2 = await send('scan ' + (s.scan ? 'on' : 'off'));
  check('scan 复原', s2.ok === true && s2.state && s2.state.scan === s.scan, JSON.stringify(s2));

  const rl = await send('reload');
  check('reload 成功', rl.ok === true, rl.err);

  const unknown = await send('definitely-not-a-command');
  check('未知命令报错', unknown.ok === false, JSON.stringify(unknown));

  console.log(failures ? `\n${failures} 项失败` : '\n全部通过');
  process.exit(failures ? 1 : 0);
}

async function main() {
  const args = process.argv.slice(2);
  if (args.length) {
    const command = args.join(' ');
    const r = await send(command);
    console.log(JSON.stringify(r, null, 2));
    process.exit(r.offline ? 2 : r.ok ? 0 : 1);
  }
  await fullSuite();
}

main();
