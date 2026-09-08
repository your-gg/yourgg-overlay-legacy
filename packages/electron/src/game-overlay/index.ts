import { GameOverlayManager } from './manager.js';
import type {
  GameId,
  GameOverlayManagerOptions,
  GameOverlayOptions,
} from './types.js';

export * from './manager.js';
export * from './processes.js';
export * from './profiles.js';
export * from './session.js';
export * from './types.js';

export async function startGameOverlays(
  options: GameOverlayManagerOptions,
): Promise<GameOverlayManager> {
  return new GameOverlayManager(options).start();
}

/**
 * Start overlays for a single game. `game` is either a built-in key
 * (`'league'`, `'valorant'`) or any string paired with `options.executable`
 * or `options.match`.
 */
export async function startGameOverlay(
  game: GameId,
  options: GameOverlayOptions,
  managerOptions: Omit<GameOverlayManagerOptions, 'games'> = {},
): Promise<GameOverlayManager> {
  return startGameOverlays({
    ...managerOptions,
    games: {
      [game]: options,
    },
  });
}
