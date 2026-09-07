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
