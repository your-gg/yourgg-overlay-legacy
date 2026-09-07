import { app, BrowserWindow } from 'electron';
import {
  Overlay,
  defaultDllDir,
  length,
  percent,
  type GpuLuid,
} from '@your-gg/yourgg-core';
import { ElectronOverlayInput } from '../input.js';
import { ElectronOverlaySurface } from '../surface.js';
import type {
  GameId,
  GameOverlayOptions,
  GameOverlaySession,
  GameProcess,
} from './types.js';

const DEFAULT_WIDTH = 520;
const DEFAULT_HEIGHT = 64;
const DEFAULT_FRAME_RATE = 30;
const DEFAULT_ATTACH_TIMEOUT = 10_000;
const DEFAULT_SURFACE_TIMEOUT = 15_000;

type AddedWindow = {
  id: number,
  luid: GpuLuid,
};

function resolveDllDir(dllDir?: string): string {
  return (dllDir ?? defaultDllDir()).replace(
    /app\.asar(?=[\\/])/,
    'app.asar.unpacked',
  );
}

function waitForAddedWindow(
  overlay: Overlay,
  timeoutMs: number,
): Promise<AddedWindow> {
  return new Promise((resolve, reject) => {
    const cleanup = () => {
      clearTimeout(timer);
      overlay.event.off('added', onAdded);
      overlay.event.off('error', onError);
      overlay.event.off('disconnected', onDisconnected);
    };
    const onAdded = (id: number, _width: number, _height: number, luid: GpuLuid) => {
      cleanup();
      resolve({ id, luid });
    };
    const onError = (error: unknown) => {
      cleanup();
      reject(
        error instanceof Error
          ? error
          : new Error('Overlay failed before a render surface was available', {
              cause: error,
            }),
      );
    };
    const onDisconnected = () => {
      cleanup();
      reject(new Error('Overlay disconnected before a render surface was available'));
    };
    const timer = setTimeout(() => {
      cleanup();
      reject(
        new Error(
          `Timed out waiting for an overlay surface after ${String(timeoutMs)}ms`,
        ),
      );
    }, timeoutMs);

    overlay.event.once('added', onAdded);
    overlay.event.once('error', onError);
    overlay.event.once('disconnected', onDisconnected);
  });
}

class ElectronGameOverlaySession implements GameOverlaySession {
  private surface: ElectronOverlaySurface | null;
  private input: ElectronOverlayInput | null = null;
  private readonly inputBlockingEndedHandler: (id: number) => void;
  private stopping = false;
  private isStopped = false;
  private interactive = false;

  constructor(
    readonly game: GameId,
    readonly process: GameProcess,
    readonly overlay: Overlay,
    readonly window: BrowserWindow,
    private readonly windowId: number,
    surface: ElectronOverlaySurface,
  ) {
    this.surface = surface;
    this.surface.events.on('error', (error) => {
      if (this.overlay.event.listenerCount('error') !== 0) {
        this.overlay.event.emit('error', error);
      } else {
        console.error(`${this.game} overlay surface error:`, error);
      }
    });
    this.overlay.event.once('disconnected', () => {
      void this.stop();
    });
    this.overlay.event.on(
      'input_blocking_ended',
      (this.inputBlockingEndedHandler = (id) => {
        if (id === this.windowId) {
          void this.disconnectInput();
        }
      }),
    );
    this.window.once('closed', () => {
      void this.stop();
    });
  }

  get stopped(): boolean {
    return this.isStopped;
  }

  async setInteractive(interactive: boolean): Promise<void> {
    if (this.isStopped || this.interactive === interactive) {
      return;
    }

    if (interactive) {
      await this.overlay.listenInput(this.windowId, true, true);
      this.input = ElectronOverlayInput.connect(
        { id: this.windowId, overlay: this.overlay },
        this.window.webContents,
      );
      try {
        await this.overlay.blockInput(this.windowId, true);
        this.interactive = true;
      } catch (error) {
        await this.input.disconnect();
        this.input = null;
        throw error;
      }
      return;
    }

    await this.overlay.blockInput(this.windowId, false);
    await this.disconnectInput();
  }

  async stop(): Promise<void> {
    if (this.stopping || this.isStopped) {
      return;
    }
    this.stopping = true;

    try {
      this.overlay.event.off(
        'input_blocking_ended',
        this.inputBlockingEndedHandler,
      );
      if (this.interactive) {
        try {
          await this.overlay.blockInput(this.windowId, false);
        } catch {
          // The target process may already be gone.
        }
      }
      await this.disconnectInput();

      try {
        await this.surface?.disconnect();
      } catch {
        // The target process may already be gone.
      }
      this.surface = null;
      this.overlay.destroy();

      if (!this.window.isDestroyed()) {
        this.window.destroy();
      }
    } finally {
      this.isStopped = true;
      this.stopping = false;
    }
  }

  private async disconnectInput(): Promise<void> {
    const input = this.input;
    this.input = null;
    this.interactive = false;
    await input?.disconnect();
  }
}

export async function createGameOverlaySession(
  game: GameId,
  process: GameProcess,
  options: GameOverlayOptions,
  dllDir: string = resolveDllDir(),
): Promise<GameOverlaySession> {
  await app.whenReady();

  const overlay = await Overlay.attach(
    resolveDllDir(dllDir || undefined),
    process.pid,
    options.attachTimeout ?? DEFAULT_ATTACH_TIMEOUT,
  );
  let window: BrowserWindow | null = null;

  try {
    const addedWindow = waitForAddedWindow(
      overlay,
      options.surfaceTimeout ?? DEFAULT_SURFACE_TIMEOUT,
    );
    const requested = options.browserWindow ?? {};
    const windowOptions = {
      ...requested,
      width: options.width ?? requested.width ?? DEFAULT_WIDTH,
      height: options.height ?? requested.height ?? DEFAULT_HEIGHT,
      show: false,
      frame: false,
      transparent: true,
      resizable: false,
      skipTaskbar: true,
      webPreferences: {
        ...requested.webPreferences,
        backgroundThrottling: false,
        offscreen: {
          useSharedTexture: true,
        },
      },
    };

    window = options.windowFactory
      ? options.windowFactory({ game, process, options: windowOptions })
      : new BrowserWindow(windowOptions);
    window.webContents.setFrameRate(options.frameRate ?? DEFAULT_FRAME_RATE);
    const [, { id, luid }] = await Promise.all([
      window.loadURL(options.url),
      addedWindow,
    ]);
    const placement = options.placement ?? {};
    await overlay.setPosition(
      id,
      placement.x ?? percent(1),
      placement.y ?? percent(0),
    );
    await overlay.setAnchor(
      id,
      placement.anchorX ?? percent(1),
      placement.anchorY ?? percent(0),
    );
    await overlay.setMargin(
      id,
      placement.margin?.top ?? length(18),
      placement.margin?.right ?? length(18),
      placement.margin?.bottom ?? length(0),
      placement.margin?.left ?? length(0),
    );

    const surface = ElectronOverlaySurface.connect(
      { id, overlay },
      luid,
      window.webContents,
    );
    window.webContents.startPainting();
    window.webContents.invalidate();

    const session = new ElectronGameOverlaySession(
      game,
      process,
      overlay,
      window,
      id,
      surface,
    );
    if (options.interactive) {
      await session.setInteractive(true);
    }
    return session;
  } catch (error) {
    overlay.destroy();
    if (window && !window.isDestroyed()) {
      window.destroy();
    }
    throw error;
  }
}
