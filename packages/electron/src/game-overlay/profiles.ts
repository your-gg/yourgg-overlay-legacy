import type {
  GameId,
  GameOverlayOptions,
  GameProcess,
  GameProfile,
  GameTarget,
} from './types.js';

/**
 * Built-in targets. A `games` key matching one of these may omit
 * `executable`/`match`; any other key must provide one of them.
 */
export const GAME_PROFILES: Readonly<Partial<Record<string, GameProfile>>> = {
  valorant: {
    id: 'valorant',
    executable: 'VALORANT-Win64-Shipping.exe',
  },
  league: {
    id: 'league',
    executable: 'League of Legends.exe',
  },
};

/**
 * Resolve how a `games` entry identifies its process: an explicit `match`
 * wins, then an explicit `executable`, then the built-in profile for the key.
 * Throws when none applies, so a misspelt key fails at construction rather
 * than silently never attaching.
 */
export function resolveGameTarget(
  game: GameId,
  options: Pick<GameOverlayOptions, 'executable' | 'match'>,
): GameTarget {
  if (options.match) {
    return { id: game, match: options.match };
  }
  const executable = options.executable ?? GAME_PROFILES[game]?.executable;
  if (!executable) {
    throw new Error(
      `Game "${game}" has no built-in profile; pass \`executable\` or \`match\` in its options`,
    );
  }
  const wanted = executable.toLowerCase();
  return {
    id: game,
    executable,
    match: ({ name }) => name.toLowerCase() === wanted,
  };
}

export function findGameProcess(
  target: GameTarget,
  processes: readonly GameProcess[],
): GameProcess | undefined {
  return processes.find(process => target.match(process));
}
