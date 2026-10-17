import { setTimeout as delay } from 'node:timers/promises';
import { run } from './common.mjs';

// DiskImages can briefly retain the image lock after `create` has returned.
// Retry only explicit resource contention; corrupt images and signing errors
// must still fail the build. The same rule covers attach/detach contention.
export async function diskImage(args) {
  for (let attempt = 0; ; attempt++) {
    try {
      const output = run('hdiutil', args, { stdio: 'pipe' });
      if (output.length) process.stdout.write(output);
      return;
    } catch (error) {
      const diagnostic = `${error.stderr || ''}\n${error.stdout || ''}`;
      if (attempt >= 4 || !/resource (?:temporarily unavailable|busy)|device busy/i.test(diagnostic)) {
        if (error.stderr?.length) process.stderr.write(error.stderr);
        throw error;
      }
      const ms = 1000 * 2 ** attempt;
      console.warn(`hdiutil ${args[0]}: temporary resource contention; retry ${attempt + 1}/4 in ${ms} ms`);
      await delay(ms);
    }
  }
}
