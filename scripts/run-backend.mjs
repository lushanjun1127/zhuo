import dotenv from 'dotenv';
import { spawn } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const projectRoot = dirname(dirname(fileURLToPath(import.meta.url)));
dotenv.config({ path: join(projectRoot, '.env') });

const command = process.platform === 'win32' ? 'cargo.exe' : 'cargo';
const child = spawn(command, ['run', '-p', 'zhuo-server'], {
  stdio: 'inherit',
  env: { ...process.env },
});

child.on('error', (error) => {
  console.error(`[backend] failed to start: ${error.message}`);
  process.exit(1);
});
child.on('exit', (code, signal) => {
  if (signal) process.kill(process.pid, signal);
  else process.exit(code ?? 0);
});

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => {
    if (!child.killed) child.kill(signal);
  });
}
