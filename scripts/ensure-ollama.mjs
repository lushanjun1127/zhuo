// 确保 Ollama 在 :11434 上可用；没跑就用本机 ollama 拉起 `ollama serve`。
// 被 concurrently 当作一个常驻任务：已运行时保持存活（退出会被 -k 连坐杀掉兄弟进程）。
import dotenv from 'dotenv';
import net from 'node:net';
import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const projectRoot = dirname(dirname(fileURLToPath(import.meta.url)));
dotenv.config({ path: join(projectRoot, '.env') });

const PORT = 11434;
const HOST = '127.0.0.1';

function portUp(timeoutMs = 1500) {
  return new Promise((resolve) => {
    const s = net.connect({ host: HOST, port: PORT, timeout: timeoutMs });
    s.once('connect', () => { s.destroy(); resolve(true); });
    s.once('error', () => resolve(false));
    s.once('timeout', () => { s.destroy(); resolve(false); });
  });
}

function resolveOllamaExe() {
  const fallback = join(
    process.env.LOCALAPPDATA ?? '',
    'Programs', 'Ollama', 'ollama.exe',
  );
  return existsSync(fallback) ? fallback : 'ollama';
}

const stayAlive = () => setInterval(() => {}, 1 << 30);

const up = await portUp();
if (up) {
  console.log(`[ollama] already running on :${PORT}`);
  console.log('[ollama] note: .env settings only apply when this script launches ollama serve');
  stayAlive();
} else {
  const exe = resolveOllamaExe();
  console.log(`[ollama] not running, starting: ${exe} serve`);
  console.log(`[ollama] env: context=${process.env.OLLAMA_CONTEXT_LENGTH ?? '(default)'}, flash_attention=${process.env.OLLAMA_FLASH_ATTENTION ?? '(default)'}, kv_cache=${process.env.OLLAMA_KV_CACHE_TYPE ?? '(default)'}`);
  const child = spawn(exe, ['serve'], {
    stdio: 'inherit',
    env: { ...process.env },
  });
  child.on('error', (e) => {
    console.error(`[ollama] failed to start: ${e.message}`);
    process.exit(1); // 触发 concurrently -k？不：退出 1 会让 -k 杀掉 backend/web。
    // 权衡：Ollama 起不来时 RP 前端照样能用后端吗——不能（chat 依赖推理）。
    // 所以失败即退出合理，用户能在窗口里直接看到原因。
  });
  child.on('exit', (code) => process.exit(code ?? 0));
  // 等端口就绪（最多 20s），就绪后提示并保持存活
  for (let i = 0; i < 20; i++) {
    if (await portUp(500)) {
      console.log(`[ollama] ready on :${PORT}`);
      break;
    }
    await new Promise((r) => setTimeout(r, 1000));
  }
  stayAlive();
}

process.on('SIGINT', () => process.exit(0));
