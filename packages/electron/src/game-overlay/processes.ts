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

export const listWindowsProcesses: GameProcessProvider = async () => {
  if (process.platform !== 'win32') {
    return [];
  }

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
