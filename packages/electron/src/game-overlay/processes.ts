import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import type { GameProcess, GameProcessProvider } from './types.js';

const execFileAsync = promisify(execFile);

export function parseTasklist(output: string): GameProcess[] {
  const processes: GameProcess[] = [];

  for (const line of output.split(/\r?\n/)) {
    const match = /^"((?:[^"]|"")*)","(\d+)"/.exec(line);
    if (!match) {
      continue;
    }

    processes.push({
      name: match[1].replaceAll('""', '"'),
      pid: Number(match[2]),
    });
  }

  return processes;
}

/**
 * Default process provider: the native Toolhelp32 snapshot exposed by
 * `@your-gg/yourgg-core` (no process spawn, ~1ms). If the core addon cannot be
 * loaded the error propagates to the manager's `error` event on purpose: the
 * same addon is needed for `Overlay.attach`, so a missing addon is a packaging
 * problem to surface, not something to paper over with a shell fallback.
 */
export const listWindowsProcesses: GameProcessProvider = async () => {
  if (process.platform !== 'win32') {
    return [];
  }

  const { listProcesses } = await import('@your-gg/yourgg-core');
  return listProcesses();
};

/**
 * Shell-based provider: spawns `tasklist.exe` and parses its CSV output. Not
 * used by default; pass it as `processProvider` explicitly if the native
 * snapshot is undesirable in a given environment.
 */
export const listProcessesViaTasklist: GameProcessProvider = async () => {
  const { stdout } = await execFileAsync(
    'tasklist.exe',
    ['/FO', 'CSV', '/NH'],
    {
      encoding: 'utf8',
      timeout: 2_000,
      windowsHide: true,
    },
  );

  return parseTasklist(stdout);
};
