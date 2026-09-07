import type { GameId, GameProfile, GameProcess } from './types.js';

export const GAME_PROFILES = {
  valorant: {
    id: 'valorant',
    executable: 'VALORANT-Win64-Shipping.exe',
  },
  league: {
    id: 'league',
    executable: 'League of Legends.exe',
  },
} as const satisfies Record<GameId, GameProfile>;

export function findGameProcess(
  game: GameId,
  processes: readonly GameProcess[],
): GameProcess | undefined {
  const executable = GAME_PROFILES[game].executable.toLowerCase();
  return processes.find(({ name }) => name.toLowerCase() === executable);
}
