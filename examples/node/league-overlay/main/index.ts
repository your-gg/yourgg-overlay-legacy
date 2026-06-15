/**
 * League of Legends overlay timing demo + diagnostic.
 *
 * Goal: show the overlay the moment your champion is controllable (like Blitz),
 * regardless of how fast the loading screen is.
 *
 * VERIFIED root cause of the "~10s" (adversarial review, 2026-06): it is NOT the
 * SetWindowsHookEx message pump (that earlier theory is wrong). `Overlay.attach`
 * (inject + IPC connect) is fast. The delay is the 'added' event — the overlay
 * only becomes READY TO DRAW when the game does a real (non-DXGI_PRESENT_TEST)
 * Present on the bound window AFTER our IPC client connects and the event sink is
 * set. That lands at a roughly FIXED point in League's render/startup timeline,
 * independent of loading length. So if loading is 4s but 'added' is ~10s, you
 * miss the controllable moment.
 *
 * The real fix lives in the Rust DLL (make ready-to-draw happen early). This
 * example is wired to MEASURE it: it logs STAGE 1 (inject + IPC) vs STAGE 2
 * (IPC -> 'added' / ready-to-draw) separately so you can see where the time goes.
 *
 * Strategy: inject as early as possible (during loading) so ready-to-draw beats
 * the controllable moment, keep the overlay hidden, then show on a precise
 * signal — the Live Client Data API `gameData.gameTime > 0` (= controllable).
 * Note: gameTime>0 lags loading, so it is the "controllable" trigger, NOT a way
 * to appear during loading. `SHOW_ON=loading` shows as soon as ready-to-draw.
 *
 * NOTE: all logs are ASCII on purpose — Windows consoles (esp. ko-KR code pages)
 * mangle UTF-8, which would corrupt the STAGE numbers we care about.
 */
import { app, BrowserWindow } from 'electron';
import { defaultDllDir, Overlay, percent, length, type GpuLuid } from '@your-gg/yourgg-core';
import { type OverlayWindow } from '@your-gg/yourgg-overlay';
import { ElectronOverlaySurface } from '@your-gg/yourgg-overlay/surface';
import find from 'find-process';
import https from 'node:https';

/** Game (not client) process. `LeagueClient.exe` is the launcher; we want the game. */
const GAME_PROCESS = process.argv[2] ?? 'League of Legends.exe';
/** Riot's in-game Live Client Data API (self-signed cert, localhost only). */
const LIVE_API = 'https://127.0.0.1:2999/liveclientdata/allgamedata';
const PROCESS_POLL_MS = 250;
const LIVE_POLL_MS = 250;
/** `ingame` (default): show when controllable. `loading`: show right after inject. */
const SHOW_ON = (process.env.SHOW_ON ?? 'ingame').toLowerCase();
/**
 * Directory containing the overlay DLLs (yourgg_overlay-x64.dll, etc).
 *
 * IMPORTANT: Vanguard blocks UNSIGNED DLLs from loading into League — that shows
 * up as STAGE 1 timing out with "ipc client wait timeout" (the hook registers
 * but the DLL never maps, so its IPC server never starts). Point DLL_DIR at the
 * SIGNED release DLLs (e.g. your working 1.1.0 app's app.asar.unpacked dir) to
 * get past STAGE 1. Defaults to the locally-built (likely unsigned) DLLs.
 */
const DLL_DIR = process.env.DLL_DIR ?? defaultDllDir().replace('app.asar', 'app.asar.unpacked');

const bootedAt = Date.now();
const stamp = () => `+${((Date.now() - bootedAt) / 1000).toFixed(2)}s`;
const log = (...args: unknown[]) => console.log(`[league-overlay ${stamp()}]`, ...args);
const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/** Subset of the Live Client Data payload we care about. */
interface LiveData {
  gameData?: {
    gameTime?: number;
  };
}

/** Find the running game pid, or null if not running. */
async function findGamePid(): Promise<number | null> {
  const list = await find('name', GAME_PROCESS, true);
  return list.length > 0 ? list[0].pid : null;
}

/** Cheap liveness check without re-scanning every process. */
function isAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (e) {
    // EPERM => exists but not ours to signal (still alive). ESRCH => gone.
    return (e as NodeJS.ErrnoException).code === 'EPERM';
  }
}

/** Poll port 2999. Resolves the parsed payload, or null on any failure. */
function fetchLiveData(): Promise<LiveData | null> {
  return new Promise((resolve) => {
    const req = https.get(LIVE_API, { rejectUnauthorized: false, timeout: 1000 }, (res) => {
      if (res.statusCode !== 200) {
        res.resume();
        resolve(null);
        return;
      }
      let body = '';
      res.setEncoding('utf8');
      res.on('data', (chunk) => (body += chunk));
      res.on('end', () => {
        try {
          resolve(JSON.parse(body) as LiveData);
        } catch {
          resolve(null);
        }
      });
    });
    req.on('error', () => resolve(null));
    req.on('timeout', () => {
      req.destroy();
      resolve(null);
    });
  });
}

/** Wait until the game process appears. */
async function waitForGameProcess(): Promise<number> {
  for (;;) {
    const pid = await findGamePid();
    if (pid !== null) return pid;
    await sleep(PROCESS_POLL_MS);
  }
}

/**
 * Inject as early as possible and resolve once the overlay surface is ready.
 *
 * Right after the process is spawned it may not have a window yet, so
 * `Overlay.attach` (which needs a visible GUI thread to hook) can fail or time
 * out. We retry until it succeeds, as long as the game is still alive.
 */
async function attachWhenReady(
  pid: number,
): Promise<{ overlay: Overlay; id: number; luid: GpuLuid }> {
  const dllDir = DLL_DIR;
  for (;;) {
    if (!isAlive(pid)) throw new Error('game exited before injection completed');
    try {
      // STAGE 1 — inject + IPC connect. Resolves once the DLL's hook has fired
      // and its named pipe is connected. Win32 semantics say this is fast.
      const tAttachStart = Date.now();
      const hb1 = setInterval(
        () => log(`  ...STAGE 1 still waiting ${((Date.now() - tAttachStart) / 1000).toFixed(1)}s (inject + IPC)`),
        2000,
      );
      let overlay: Overlay;
      try {
        overlay = await Overlay.attach(dllDir, pid, 30_000);
      } finally {
        clearInterval(hb1);
      }
      log(`  STAGE 1 (inject + IPC connect): ${Date.now() - tAttachStart}ms`);

      // STAGE 2 — ready to draw. 'added' fires only when the game does a real
      // (non-TEST) Present on the bound window AFTER our IPC client connected.
      // This is the suspected ~10s; it tracks League's render timeline, not the
      // loading bar. Measure it here.
      const tAddedWait = Date.now();
      const hb2 = setInterval(
        () => log(`  ...STAGE 2 still waiting ${((Date.now() - tAddedWait) / 1000).toFixed(1)}s (waiting 'added')`),
        2000,
      );
      let id: number;
      let luid: GpuLuid;
      try {
        [id, luid] = await new Promise<[number, GpuLuid]>((resolve) => {
          overlay.event.once('added', (winId, _w, _h, gpuLuid) => resolve([winId, gpuLuid]));
        });
      } finally {
        clearInterval(hb2);
      }
      log(`  STAGE 2 (IPC -> 'added' / ready to draw): ${Date.now() - tAddedWait}ms  <-- suspected ~10s`);
      return { overlay, id, luid };
    } catch (err) {
      // Window not up yet / hook hasn't fired in time — back off and retry.
      log('attach retry:', err instanceof Error ? err.message : err);
      await sleep(300);
    }
  }
}

/** Poll the Live Client Data API until the champion is controllable. */
async function waitUntilControllable(pid: number): Promise<number> {
  for (;;) {
    if (!isAlive(pid)) throw new Error('game exited before becoming controllable');
    const data = await fetchLiveData();
    const gameTime = data?.gameData?.gameTime;
    if (typeof gameTime === 'number' && gameTime > 0) return gameTime;
    await sleep(LIVE_POLL_MS);
  }
}

/** Self-contained overlay UI (no renderer build step needed). */
const OVERLAY_HTML =
  'data:text/html,' +
  encodeURIComponent(`<!doctype html><html><head><meta charset="utf-8"><style>
  html,body{margin:0;padding:0;background:transparent;overflow:hidden;
    font-family:'Segoe UI',system-ui,sans-serif}
  .card{margin:8px;padding:12px 16px;border-radius:12px;
    background:rgba(10,12,20,0.80);color:#fff;
    box-shadow:0 6px 24px rgba(0,0,0,.55);border:1px solid rgba(122,162,255,.45)}
  .title{font-weight:700;font-size:16px;color:#7aa2ff;letter-spacing:.3px}
  .sub{font-size:12px;opacity:.85;margin-top:5px}
</style></head><body>
  <div class="card">
    <div class="title">YOUR.GG OVERLAY</div>
    <div class="sub" id="t">active</div>
  </div>
  <script>
    const shownAt=Date.now();
    setInterval(()=>{document.getElementById('t').textContent=
      'shown '+((Date.now()-shownAt)/1000).toFixed(1)+'s ago';},100);
  </script>
</body></html>`);

/** Build the offscreen browser that renders the overlay, kept hidden until shown. */
async function createHiddenOverlayWindow(): Promise<BrowserWindow> {
  const win = new BrowserWindow({
    width: 360,
    height: 96,
    show: false,
    webPreferences: {
      offscreen: {
        useSharedTexture: true,
      },
    },
  });
  // Hidden = not painting to the surface yet.
  win.webContents.stopPainting();
  await win.loadURL(OVERLAY_HTML);
  return win;
}

/** Run one full game session: detect -> inject (hidden) -> show on trigger -> cleanup. */
async function runSession(): Promise<void> {
  log(`waiting for '${GAME_PROCESS}' process...`);
  const pid = await waitForGameProcess();
  const tProcess = Date.now();
  log(`process detected! pid=${pid}`);

  // --- Phase 1: inject early, keep hidden ---
  log('injecting (arm early during loading, stays hidden)...');
  const { overlay, id, luid } = await attachWhenReady(pid);
  const tInjected = Date.now();
  log(`ready to draw. detect -> ready: ${((tInjected - tProcess) / 1000).toFixed(2)}s`);

  const win = await createHiddenOverlayWindow();
  const overlayWindow: OverlayWindow = { id, overlay };
  // Top-center of the game window.
  void overlay.setPosition(id, percent(0.5), length(24));
  void overlay.setAnchor(id, percent(0.5), percent(0));

  // --- Phase 2: decide when to show ---
  if (SHOW_ON === 'loading') {
    log("SHOW_ON=loading -> show as soon as ready to draw");
  } else {
    log('waiting for in-game (controllable) signal... (port 2999 polling)');
    const gameTime = await waitUntilControllable(pid);
    log(`in-game detected! gameTime=${gameTime.toFixed(1)}s`);
  }

  // --- Phase 3: show (instant — injection already done) ---
  const tTrigger = Date.now();
  const surface = ElectronOverlaySurface.connect(overlayWindow, luid, win.webContents);
  surface.events.on('error', (e) => log('surface error:', e));
  win.webContents.startPainting();
  win.webContents.invalidate();
  log(`overlay shown! trigger -> shown: ${Date.now() - tTrigger}ms`);

  // --- wait for the game to exit, then clean up and loop for the next game ---
  while (isAlive(pid)) await sleep(1000);
  log('game exited. cleaning up, waiting for next game.');
  await surface.disconnect().catch(() => {});
  overlay.destroy();
  win.destroy();
}

async function main(): Promise<void> {
  await app.whenReady();
  log(`start. target process='${GAME_PROCESS}', SHOW_ON=${SHOW_ON}`);
  log(`DLL_DIR=${DLL_DIR}`);
  for (;;) {
    try {
      await runSession();
    } catch (err) {
      log('session error:', err instanceof Error ? err.message : err);
      await sleep(1000);
    }
  }
}

main().catch((err: unknown) => {
  app.quit();
  throw err;
});
