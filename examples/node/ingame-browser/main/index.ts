import { app } from 'electron';
import {
  startGameOverlays,
  type GameId,
  type GameOverlayOptions,
} from '@your-gg/yourgg-overlay';

async function main() {
  await app.whenReady();

  const selected = process.argv[2] ?? 'all';
  if (!['valorant', 'league', 'all'].includes(selected)) {
    throw new Error('Usage: ingame-browser [valorant|league|all]');
  }

  const overlayPage: GameOverlayOptions = {
    url: 'https://electronjs.org',
    width: 800,
    height: 600,
  };
  const games: Partial<Record<GameId, GameOverlayOptions>> = {};
  if (selected === 'valorant' || selected === 'all') {
    games.valorant = overlayPage;
  }
  if (selected === 'league' || selected === 'all') {
    games.league = overlayPage;
  }

  // No LCU signal in this demo, so poll for the game processes.
  const manager = await startGameOverlays({ games, scanIntervalMs: 3_000 });
  manager.events.on('attached', ({ game, process }) => {
    console.log(`Attached ${game} overlay to PID ${String(process.pid)}`);
  });
  manager.events.on('detached', (game, pid) => {
    console.log(`Detached ${game} overlay from PID ${String(pid)}`);
  });
  manager.events.on('error', (game, error) => {
    console.error(`${game} overlay error:`, error);
  });

  app.on('before-quit', () => {
    void manager.stop();
  });
}

main().catch((e: unknown) => {
  app.quit();
  throw e;
});
