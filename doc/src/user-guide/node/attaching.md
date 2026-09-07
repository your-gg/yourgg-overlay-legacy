# Attaching to target process
To control overlay, you first need to attach overlay dll to target process.
The `@asdf-overlay/core` package provides a function for attaching overlay and initialize IPC connection.

Following code connects overlay to target process.
```typescript
import { defaultDllDir, Overlay } from '@asdf-overlay/core';

const overlay = await Overlay.attach(
  defaultDllDir().replace('app.asar', 'app.asar.unpacked'),
  /* target process id */ 12345,
  /* optional timeout in ms */ 5000,
);
```
Some caveats included below:
1. `@asdf-overlay/core` package includes overlay dll files for x64, ia32 and arm64 architectures.
   Asdf overlay will choose appropriate one based on the target process architecture.
   The `defaultDllDir` function returns path to the directory containing these dll files.
2. Onced injected, the dll will not be unloaded until the target process exits and maybe reused later if another connection is established.
3. If optional timeout is not provided, it will wait indefinitely.
4. The `Overlay.attach` function returns an `Overlay` instance upon successful attachment and can be used to control the overlay.
5. On Electron, `@asdf-overlay/core` must be specified as external to work correctly.
6. On Electron, `defaultDllDir` function may return path inside `app.asar` archive.
   In such cases, you need to replace `app.asar` with `app.asar.unpacked` to access the dll files.

## Attaching to VALORANT and League of Legends

`@your-gg/yourgg-overlay` provides a higher-level Electron main-process API
which discovers the game processes, creates one offscreen window per game, and
reattaches when either game restarts.

```typescript
import { app } from 'electron';
import { startGameOverlays } from '@your-gg/yourgg-overlay';

await app.whenReady();

const overlays = await startGameOverlays({
  games: {
    valorant: {
      url: 'app://overlay/valorant',
      width: 520,
      height: 64,
    },
    league: {
      url: 'app://overlay/league',
      width: 520,
      height: 64,
    },
  },
});

// Input blocking is opt-in and is available after a game attaches.
overlays.events.on('attached', (session) => {
  if (session.game === 'league') {
    void session.setInteractive(true);
  }
});

app.on('before-quit', () => {
  void overlays.stop();
});
```

Use `startGameOverlay('valorant', options)` when only one game is required.
Importing the package never starts process discovery or DLL injection; only the
`startGameOverlay` and `startGameOverlays` calls do.

The default layout is top-right with an 18-pixel margin. Supply `placement`,
`browserWindow`, or `windowFactory` to customize layout, preload scripts, and
other `BrowserWindow` details. Native APIs must remain in the Electron main
process; expose only narrow application-specific commands to a renderer through
a context-isolated preload.

### Electron packaging

- Keep `@your-gg/yourgg-core` external in Electron/Vite/Rollup configuration.
- Unpack `@your-gg/yourgg-core` native `.node`, `.dll`, and `.exe` files from
  ASAR. The high-level API corrects `app.asar` to `app.asar.unpacked` at runtime,
  but the packager must still copy those files there.
- With electron-builder, include
  `node_modules/@your-gg/yourgg-core/**/*.{node,dll,exe}` in `asarUnpack`.

### Riot Vanguard

This backend injects a rendering DLL into the target process. Repository testing
has already observed Riot Vanguard account enforcement even with a signed DLL.
There is no allow-list path for this technique. Treat this backend as explicit
R&D opt-in, not a production-safe Riot integration; a production release should
use a separate topmost window and supported Riot APIs instead.
