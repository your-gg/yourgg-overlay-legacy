import type {
  BrowserWindow,
  BrowserWindowConstructorOptions,
} from 'electron';
import type { Overlay, PercentLength } from '@your-gg/yourgg-core';

export type GameId = 'valorant' | 'league';

export type GameProcess = {
  pid: number,
  name: string,
};

export type GameProfile = {
  id: GameId,
  executable: string,
};

export type GameOverlayPlacement = {
  x?: PercentLength,
  y?: PercentLength,
  anchorX?: PercentLength,
  anchorY?: PercentLength,
  margin?: {
    top?: PercentLength,
    right?: PercentLength,
    bottom?: PercentLength,
    left?: PercentLength,
  },
};

export type GameOverlayWindowContext = {
  game: GameId,
  process: GameProcess,
  options: BrowserWindowConstructorOptions,
};

export type GameOverlayOptions = {
  url: string,
  width?: number,
  height?: number,
  frameRate?: number,
  attachTimeout?: number,
  surfaceTimeout?: number,
  placement?: GameOverlayPlacement,
  interactive?: boolean,
  browserWindow?: BrowserWindowConstructorOptions,
  windowFactory?: (context: GameOverlayWindowContext) => BrowserWindow,
};

export type GameOverlaySession = {
  readonly game: GameId,
  readonly process: GameProcess,
  readonly overlay: Overlay,
  readonly window: BrowserWindow,
  readonly stopped: boolean,
  setInteractive(interactive: boolean): Promise<void>,
  stop(): Promise<void>,
};

export type GameProcessProvider = () => Promise<readonly GameProcess[]>;

export type GameOverlaySessionFactory = (
  game: GameId,
  process: GameProcess,
  options: GameOverlayOptions,
  dllDir: string,
) => Promise<GameOverlaySession>;

export type GameOverlayManagerOptions = {
  games: Partial<Record<GameId, GameOverlayOptions>>,
  /**
   * Opt-in periodic re-scan of running processes, in milliseconds.
   *
   * Unset (or `0`) means the manager never polls: it scans once on `start()`
   * and afterwards only when the host calls `GameOverlayManager.scan()`, e.g.
   * on the LCU `/lol-gameflow/v1/gameflow-phase` event reaching `InProgress`.
   * Set e.g. `3000` to poll instead, for hosts without such a signal. Game exit
   * is detected either way through the overlay IPC disconnecting.
   */
  scanIntervalMs?: number,
  dllDir?: string,
  processProvider?: GameProcessProvider,
  sessionFactory?: GameOverlaySessionFactory,
};

export type GameOverlayManagerEvents = {
  attached: [session: GameOverlaySession],
  detached: [game: GameId, pid: number],
  error: [game: GameId, error: unknown],
};
