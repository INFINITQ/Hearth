import { describe, expect, it } from 'vitest';
import { formatSize, kindOf, readAttachment } from './attachments';

describe('attachments', () => {
  it('know a file by its type or name', () => {
    expect(kindOf('photo.JPG', '')).toBe('image');
    expect(kindOf('shot', 'image/png')).toBe('image');
    expect(kindOf('logo.svg', 'image/svg+xml')).toBe('text');
    expect(kindOf('paper.pdf', 'application/pdf')).toBe('pdf');
    expect(kindOf('main.rs', '')).toBe('text');
    expect(kindOf('notes.md', 'text/markdown')).toBe('text');
    expect(kindOf('Makefile', '')).toBe('text');
    expect(kindOf('data.json', 'application/json')).toBe('text');
    expect(kindOf('setup.exe', 'application/x-msdownload')).toBeNull();
    expect(kindOf('archive.zip', 'application/zip')).toBeNull();
  });

  it('show sizes the way people read them', () => {
    expect(formatSize(512)).toBe('512 B');
    expect(formatSize(12_300)).toBe('12 KB');
    expect(formatSize(3_500_000)).toBe('3.3 MB');
  });

  it('read text files, and refuse binary ones and unknown types', async () => {
    const text = new File(['fn main() {}'], 'main.rs', { type: '' });
    await expect(readAttachment(text)).resolves.toMatchObject({
      kind: 'text',
      name: 'main.rs',
      text: 'fn main() {}',
    });
    const binary = new File(['a\u0000b'], 'blob.txt', { type: 'text/plain' });
    await expect(readAttachment(binary)).rejects.toThrow("isn't a text file");
    const zip = new File(['PK'], 'a.zip', { type: 'application/zip' });
    await expect(readAttachment(zip)).rejects.toThrow('Hearth can attach');
  });
});
