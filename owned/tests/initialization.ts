import { Engine } from 'debugger-sh';
import { expect, test, vi } from 'vitest';

test('shares a failed initialization, then retries after the failure is cleared', async () => {
  const compile = vi
    .spyOn(WebAssembly, 'compile')
    .mockRejectedValueOnce(new Error('cold load failed'));
  try {
    await Promise.all([
      expect(Engine.create()).rejects.toThrow('cold load failed'),
      expect(Engine.create()).rejects.toThrow('cold load failed')
    ]);
    expect(compile).toHaveBeenCalledTimes(1);
  } finally {
    compile.mockRestore();
  }
  const engine = await Engine.create({ fs: { 'retry.txt': 'recovered' } });
  try {
    expect(await engine.fs.readFile('/retry.txt', 'utf8')).toBe('recovered');
  } finally {
    engine.dispose();
  }
});
