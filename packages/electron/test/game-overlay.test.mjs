import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import test, { mock } from 'node:test';
import { GameOverlayManager } from '../lib/game-overlay/manager.js';
import { parseTasklist } from '../lib/game-overlay/processes.js';
import { resolveGameTarget } from '../lib/game-overlay/profiles.js';

void test('parseTasklist parses Windows CSV output', () => {
  assert.deepEqual(
    parseTasklist(
      [
        '"VALORANT-Win64-Shipping.exe","1234","Console","1","1,000 K"',
        '"League of Legends.exe","5678","Console","1","2,000 K"',
        'INFO: No tasks are running which match the specified criteria.',
      ].join('\r\n'),
    ),
    [
      { name: 'VALORANT-Win64-Shipping.exe', pid: 1234 },
      { name: 'League of Legends.exe', pid: 5678 },
    ],
  );
});

void test('manager attaches both games and reconciles process changes', async () => {
  let processes = [
    { name: 'VALORANT-Win64-Shipping.exe', pid: 100 },
    { name: 'League of Legends.exe', pid: 200 },
  ];
  const created = [];
  const stopped = [];

  const manager = new GameOverlayManager({
    games: {
      valorant: { url: 'https://example.test/valorant' },
      league: { url: 'https://example.test/league' },
    },
    dllDir: 'C:\\overlay',
    scanIntervalMs: 60_000,
    processProvider: () => Promise.resolve(processes),
    sessionFactory: (game, process) => {
      created.push([game, process.pid]);
      const event = new EventEmitter();
      return Promise.resolve({
        game,
        process,
        // The manager only consumes the typed event emitter from Overlay.
        // @ts-expect-error Minimal native overlay test double.
        overlay: { event },
        window: {},
        stopped: false,
        setInteractive: () => Promise.resolve(),
        stop: () => {
          stopped.push([game, process.pid]);
          event.emit('disconnected');
          return Promise.resolve();
        },
      });
    },
  });

  await manager.start();
  assert.deepEqual(new Set(manager.activeGames), new Set(['valorant', 'league']));
  assert.deepEqual(created, [
    ['valorant', 100],
    ['league', 200],
  ]);

  await manager.scan();
  assert.equal(created.length, 2, 'must not attach the same PID twice');

  processes = [
    { name: 'VALORANT-Win64-Shipping.exe', pid: 101 },
    { name: 'RiotClientServices.exe', pid: 300 },
  ];
  await manager.scan();
  assert.equal(manager.getSession('valorant')?.process.pid, 101);
  assert.equal(manager.getSession('league'), undefined);
  assert.deepEqual(created.at(-1), ['valorant', 101]);
  assert.ok(stopped.some(([game, pid]) => game === 'valorant' && pid === 100));
  assert.ok(stopped.some(([game, pid]) => game === 'league' && pid === 200));

  await manager.stop();
  assert.ok(stopped.some(([game, pid]) => game === 'valorant' && pid === 101));
  assert.deepEqual(manager.activeGames, []);
});

void test('stop prevents a pending scan from attaching later', async () => {
  let releaseProcessScan;
  let attachCount = 0;
  const processScan = new Promise((resolve) => {
    releaseProcessScan = resolve;
  });
  const manager = new GameOverlayManager({
    games: {
      valorant: { url: 'https://example.test/valorant' },
    },
    dllDir: 'C:\\overlay',
    processProvider: () => processScan,
    sessionFactory: () => {
      attachCount += 1;
      return Promise.reject(new Error('must not attach after stop'));
    },
  });

  const starting = manager.start();
  await Promise.resolve();
  await manager.stop();
  releaseProcessScan([
    { name: 'VALORANT-Win64-Shipping.exe', pid: 100 },
  ]);
  await starting;

  assert.equal(attachCount, 0);
});

void test('periodic scanning is off by default', async () => {
  mock.timers.enable({ apis: ['setInterval'] });
  try {
    let scans = 0;
    const manager = new GameOverlayManager({
      games: {
        league: { url: 'https://example.test/league' },
      },
      dllDir: 'C:\\overlay',
      processProvider: () => {
        scans += 1;
        return Promise.resolve([]);
      },
      sessionFactory: () => Promise.reject(new Error('no process, must not attach')),
    });

    await manager.start();
    assert.equal(scans, 1, 'start() performs one immediate scan');

    mock.timers.tick(60_000);
    assert.equal(scans, 1, 'no timer-driven scans');

    await manager.scan();
    assert.equal(scans, 2, 'external scan() still works');

    await manager.stop();
  } finally {
    mock.timers.reset();
  }
});

void test('positive scanIntervalMs keeps periodic scanning', async () => {
  mock.timers.enable({ apis: ['setInterval'] });
  try {
    let scans = 0;
    const manager = new GameOverlayManager({
      games: {
        league: { url: 'https://example.test/league' },
      },
      dllDir: 'C:\\overlay',
      scanIntervalMs: 1_000,
      processProvider: () => {
        scans += 1;
        return Promise.resolve([]);
      },
      sessionFactory: () => Promise.reject(new Error('no process, must not attach')),
    });

    await manager.start();
    assert.equal(scans, 1);

    mock.timers.tick(3_000);
    assert.equal(scans, 4, 'one scan per interval');

    await manager.stop();
  } finally {
    mock.timers.reset();
  }
});

void test('custom executable and match targets attach alongside presets', async () => {
  const processes = [
    { name: 'League of Legends.exe', pid: 10, path: 'C:\\Riot Games\\League of Legends\\Game\\League of Legends.exe' },
    { name: 'League of Legends.exe', pid: 11, path: 'D:\\Riot Games\\League of Legends (loltmnt04)\\Game\\League of Legends.exe' },
    { name: 'SomeGame.exe', pid: 12 },
  ];
  const created = [];
  const manager = new GameOverlayManager({
    games: {
      'league': {
        url: 'https://example.test/league',
        match: p => p.name === 'League of Legends.exe' && !p.path?.includes('loltmnt'),
      },
      'league-tournament': {
        url: 'https://example.test/tournament',
        match: p => p.name === 'League of Legends.exe' && Boolean(p.path?.includes('loltmnt')),
      },
      'somegame': { url: 'https://example.test/somegame', executable: 'somegame.EXE' },
    },
    dllDir: 'C:\\overlay',
    processProvider: () => Promise.resolve(processes),
    sessionFactory: (game, process) => {
      created.push([game, process.pid]);
      return Promise.resolve({
        game,
        process,
        // @ts-expect-error Minimal native overlay test double.
        overlay: { event: new EventEmitter() },
        window: {},
        stopped: false,
        setInteractive: () => Promise.resolve(),
        stop: () => Promise.resolve(),
      });
    },
  });

  await manager.start();
  assert.deepEqual(new Set(created.map(([g, p]) => `${String(g)}:${String(p)}`)), new Set([
    'league:10',
    'league-tournament:11',
    'somegame:12',
  ]));
  await manager.stop();
});

void test('unknown game key without executable or match fails at construction', () => {
  assert.throws(
    () => new GameOverlayManager({
      games: { tft: { url: 'https://example.test/tft' } },
      dllDir: 'C:\\overlay',
    }),
    /no built-in profile/,
  );
  assert.equal(resolveGameTarget('league', {}).executable, 'League of Legends.exe');
  assert.equal(
    resolveGameTarget('league', { executable: 'Custom.exe' }).executable,
    'Custom.exe',
  );
});
