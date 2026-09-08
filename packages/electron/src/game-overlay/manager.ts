import { EventEmitter } from 'node:events';
import { findGameProcess } from './profiles.js';
import { listWindowsProcesses } from './processes.js';
import type {
  GameId,
  GameOverlayManagerEvents,
  GameOverlayManagerOptions,
  GameOverlaySession,
  GameOverlaySessionFactory,
  GameProcess,
} from './types.js';

const GAME_IDS = ['valorant', 'league'] as const;

const defaultSessionFactory: GameOverlaySessionFactory = async (
  game,
  process,
  options,
  dllDir,
) => {
  const { createGameOverlaySession } = await import('./session.js');
  return createGameOverlaySession(game, process, options, dllDir);
};

export class GameOverlayManager {
  readonly events = new EventEmitter<GameOverlayManagerEvents>();

  private readonly sessions = new Map<GameId, GameOverlaySession>();
  private readonly attaching = new Map<GameId, Promise<void>>();
  private readonly desiredPids = new Map<GameId, number>();
  private readonly processProvider;
  private readonly sessionFactory;
  private readonly dllDir;
  private readonly scanIntervalMs;
  private timer: NodeJS.Timeout | null = null;
  private started = false;
  private stopped = false;

  constructor(private readonly options: GameOverlayManagerOptions) {
    // EventEmitter treats an unhandled `error` event as an exception. Consumers
    // may subscribe for diagnostics, but retries must remain safe without one.
    this.events.on('error', () => {});
    this.processProvider = options.processProvider ?? listWindowsProcesses;
    this.sessionFactory = options.sessionFactory ?? defaultSessionFactory;
    this.dllDir = (options.dllDir ?? '').replace(
      /app\.asar(?=[\\/])/,
      'app.asar.unpacked',
    );
    this.scanIntervalMs = options.scanIntervalMs ?? 0;
  }

  getSession(game: GameId): GameOverlaySession | undefined {
    return this.sessions.get(game);
  }

  get activeGames(): readonly GameId[] {
    return [...this.sessions.keys()];
  }

  async start(): Promise<this> {
    if (this.started) {
      return this;
    }
    if (this.stopped) {
      throw new Error('A stopped GameOverlayManager cannot be restarted');
    }

    this.started = true;
    await this.scan();
    this.scheduleScan();
    return this;
  }

  async scan(): Promise<void> {
    if (this.shouldStop()) {
      return;
    }

    let processes: readonly GameProcess[];
    try {
      processes = await this.processProvider();
    } catch (error) {
      for (const game of GAME_IDS) {
        if (this.options.games[game]) {
          this.events.emit('error', game, error);
        }
      }
      return;
    }
    if (this.shouldStop()) {
      return;
    }

    await Promise.all(
      GAME_IDS.map(async (game) => {
        if (!this.options.games[game]) {
          return;
        }
        await this.reconcile(game, findGameProcess(game, processes));
      }),
    );
  }

  async setInteractive(game: GameId, interactive: boolean): Promise<void> {
    const session = this.sessions.get(game);
    if (!session) {
      throw new Error(`${game} overlay is not attached`);
    }
    await session.setInteractive(interactive);
  }

  async stop(): Promise<void> {
    if (this.stopped) {
      return;
    }
    this.stopped = true;
    this.desiredPids.clear();

    if (this.timer) {
      clearInterval(this.timer);
      this.timer = null;
    }

    const active = [...this.sessions.entries()];
    this.sessions.clear();
    await Promise.allSettled(
      active.map(async ([game, session]) => {
        await session.stop();
        this.events.emit('detached', game, session.process.pid);
      }),
    );
    await Promise.allSettled(this.attaching.values());
    this.attaching.clear();
  }

  private scheduleScan(): void {
    // Periodic scanning is opt-in. By default the host drives `scan()` from
    // its own signal (e.g. the LCU gameflow phase) instead of polling.
    if (this.stopped || this.scanIntervalMs <= 0) {
      return;
    }
    this.timer = setInterval(() => {
      void this.scan();
    }, this.scanIntervalMs);
  }

  private shouldStop(): boolean {
    return this.stopped;
  }

  private async reconcile(
    game: GameId,
    process: GameProcess | undefined,
  ): Promise<void> {
    if (this.stopped) {
      return;
    }
    const current = this.sessions.get(game);

    if (!process) {
      this.desiredPids.delete(game);
      if (current) {
        this.sessions.delete(game);
        await current.stop();
        this.events.emit('detached', game, current.process.pid);
      }
      return;
    }

    this.desiredPids.set(game, process.pid);
    if (current?.process.pid === process.pid && !current.stopped) {
      return;
    }
    if (current) {
      this.sessions.delete(game);
      await current.stop();
      this.events.emit('detached', game, current.process.pid);
    }
    if (this.attaching.has(game)) {
      return;
    }

    const pending = this.attach(game, process).finally(() => {
      this.attaching.delete(game);
    });
    this.attaching.set(game, pending);
    await pending;
  }

  private async attach(game: GameId, process: GameProcess): Promise<void> {
    const options = this.options.games[game];
    if (!options) {
      return;
    }

    try {
      const session = await this.sessionFactory(
        game,
        process,
        options,
        this.dllDir,
      );
      if (this.stopped || this.desiredPids.get(game) !== process.pid) {
        await session.stop();
        return;
      }

      this.sessions.set(game, session);
      const onError = (error: unknown) => {
        this.events.emit('error', game, error);
      };
      session.overlay.event.on('error', onError);
      session.overlay.event.once('disconnected', () => {
        session.overlay.event.off('error', onError);
        if (this.sessions.get(game) !== session) {
          return;
        }
        this.sessions.delete(game);
        this.events.emit('detached', game, process.pid);
      });
      this.events.emit('attached', session);
    } catch (error) {
      this.events.emit('error', game, error);
    }
  }
}
