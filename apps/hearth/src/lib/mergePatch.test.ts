import { describe, expect, it } from 'vitest';
import { mergePatch } from './mergePatch';

describe('mergePatch', () => {
  const settings = {
    theme: 'dark',
    runtime: { llamaServerPath: null, modelFolders: ['D:\\A'], presets: { a: { context: 8192 } } },
  };

  it('merges nested objects instead of replacing them', () => {
    const next = mergePatch(settings, { runtime: { llamaServerPath: 'C:\\x.exe' } });
    expect(next.runtime).toEqual({
      llamaServerPath: 'C:\\x.exe',
      modelFolders: ['D:\\A'],
      presets: { a: { context: 8192 } },
    });
    expect(settings.runtime.llamaServerPath).toBeNull();
  });

  it('replaces arrays, and null removes nested keys but not top-level ones', () => {
    const next = mergePatch(settings, {
      theme: null,
      runtime: { modelFolders: ['D:\\B'], presets: { a: null, b: { context: 4096 } } },
    });
    expect(next.theme).toBe('dark');
    expect(next.runtime.modelFolders).toEqual(['D:\\B']);
    expect(next.runtime.presets).toEqual({ b: { context: 4096 } });
  });
});
