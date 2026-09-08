import type {
  BrowserWindow,
  BrowserWindowConstructorOptions,
} from 'electron';
import type { Overlay, PercentLength } from '@your-gg/yourgg-core';

/**
 * Key identifying a target game in `games`. `'league'` and `'valorant'` have
 * built-in profiles; any other string works when the entry supplies
 * `executable` or `match`.
 */
export type GameId = string;

export type GameProcess = {
  pid: number,
  /**
   * Executable file name without path, e.g. `League of Legends.exe`.
   */
  name: string,
  /**
   * Full executable path when the process could be queried. Use it to tell
   * apart installs that share an executable name (live vs tournament client).
   */
  path?: string,
};

export type GameProfile = {
  id: GameId,
  executable: string,
};

/**
 * Resolved description of how one `games` entry finds its process.
 */
export type GameTarget = {
  id: GameId,
  executable?: string,
  match: (process: GameProcess) => boolean,
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
  /**
   * Executable file name to attach to (case-insensitive). Optional for keys
   * with a built-in profile (`league`, `valorant`), required otherwise unless
   * `match` is given.
   */
  executable?: string,
  /**
   * Custom process predicate. Takes precedence over `executable` and the
   * built-in profile. Use when the executable name alone is ambiguous, e.g.
   * `(p) => p.name === 'League of Legends.exe' && p.path?.includes('loltmnt')`.
   */
  match?: (process: GameProcess) => boolean,
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
