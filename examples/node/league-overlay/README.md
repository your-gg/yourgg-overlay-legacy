# league-overlay

A timing-focused League of Legends overlay demo: it shows the overlay the exact
moment your champion becomes controllable (like Blitz), instead of ~10s after a
game-start event.

## The idea

**Verified root cause of the "~10s" (adversarial review, 2026-06):** it is *not*
the `SetWindowsHookEx` message pump. That hook fires within ~1 frame, and
`Overlay.attach` (inject + IPC connect) is fast. The real delay is the **'added'
event** — the overlay only becomes *ready to draw* when the game does a real
(non-`DXGI_PRESENT_TEST`) `Present` on the bound window **after** our IPC client
connects and the event sink is set. That lands at a roughly **fixed** point in
League's render/startup timeline, independent of how long the loading bar takes.
So if loading is 4s but 'added' is ~10s, you miss the controllable moment — which
is exactly the reported symptom.

This demo is therefore both a **diagnostic** and a **decoupled architecture**:

1. **Inject as early as possible, hidden.** The instant `League of Legends.exe`
   has a window (the loading screen), it attaches and keeps the overlay hidden.
2. **Measure the two stages separately** so you can see where the time actually
   goes: STAGE 1 (inject + IPC connect) vs STAGE 2 (IPC → 'added' / ready to
   draw). STAGE 2 is the suspected ~10s.
3. **Show on a precise signal.** It polls the in-game Live Client Data API at
   `https://127.0.0.1:2999/liveclientdata/allgamedata` and shows the overlay the
   instant `gameData.gameTime > 0` (= champion controllable). This only appears
   on time **if** ready-to-draw (STAGE 2) finishes before that moment — which is
   why the real fix is making STAGE 2 fast (in the Rust DLL), not nudging harder.

## Run

Build the workspace first (needs the Rust toolchain — same as building the
published package), then start the example:

```bash
pnpm build
pnpm --filter league-overlay start
```

Then start a League game. Watch the console timestamps — this is the measurement
that tells us where the ~10s actually is:

- `프로세스 감지` → game process found
- `STAGE 1 (주입+IPC 연결): Nms` → inject + IPC connect (expected: fast)
- `STAGE 2 (IPC→added/그릴 준비): Nms` → time until ready to draw (**suspected ~10s**)
- `인게임 감지` → port 2999 reports `gameTime > 0` (controllable)
- `오버레이 표시! 트리거→표시: Nms` → shown after the trigger

If STAGE 2 is the big number, the fix is in the Rust DLL's present/sink gating,
not in the injector.

### Compare with the old behavior

```bash
SHOW_ON=loading pnpm --filter league-overlay start
```

`SHOW_ON=loading` shows the overlay right after injection (the old, coupled
behavior) so you can feel the difference — it appears mid-loading at an
unpredictable time, instead of exactly when you can move.

### Signed DLLs (Vanguard)

League runs Riot **Vanguard**, which **blocks unsigned DLLs** from loading into
the game process. With a locally-built (unsigned) DLL, injection fails as STAGE 1
timing out:

```
attach retry: cannot inject to the process
Caused by:
    ipc client wait timeout
```

The hook registers, but Vanguard refuses to map the DLL, so its IPC server never
starts. Point `DLL_DIR` at the **signed** release DLLs to get past this — e.g.
the `yourgg_overlay-x64.dll` shipped inside your working signed app
(`resources/app.asar.unpacked/node_modules/@your-gg/yourgg-core/`):

```powershell
$env:DLL_DIR = "C:\path\to\signed\dlls"
pnpm --filter league-overlay dev
```

Note: any future Rust DLL fix must also be signed before it can be tested against
real League. To iterate on overlay/timing logic without signing, test against a
non-Vanguard DirectX app first.

### Options

- First CLI arg overrides the target process name (default
  `League of Legends.exe`).
- `SHOW_ON=ingame` (default) shows on the controllable signal; `SHOW_ON=loading`
  shows right after injection.
- `DLL_DIR` overrides where the overlay DLLs are loaded from (use signed DLLs for
  League — see above).
