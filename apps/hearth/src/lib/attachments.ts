// Files the user attaches: text and code files as text, a PDF's text (pdf.js, loaded only when
// needed), and images resized for vision models. What comes out is what Rust saves with the
// message.

import type { NewAttachment } from './types';

export const MAX_FILES = 10;
const MAX_BYTES = 25 * 1024 * 1024;
/** Rust cuts longer text too (with a note); this keeps big files from crossing over whole. */
const MAX_TEXT_CHARS = 400_000;
/** Vision models scale images down to about this anyway. */
const IMAGE_SIDE = 1568;

const TEXT_EXTENSIONS = new Set([
  'txt',
  'md',
  'markdown',
  'rst',
  'csv',
  'tsv',
  'json',
  'jsonl',
  'yaml',
  'yml',
  'toml',
  'ini',
  'cfg',
  'conf',
  'xml',
  'html',
  'htm',
  'css',
  'scss',
  'less',
  'js',
  'mjs',
  'cjs',
  'jsx',
  'ts',
  'tsx',
  'py',
  'rs',
  'go',
  'java',
  'kt',
  'kts',
  'c',
  'h',
  'cpp',
  'hpp',
  'cc',
  'cs',
  'rb',
  'php',
  'swift',
  'sql',
  'sh',
  'bash',
  'zsh',
  'ps1',
  'bat',
  'cmd',
  'lua',
  'r',
  'dart',
  'scala',
  'vue',
  'svelte',
  'log',
  'env',
  'gitignore',
  'dockerfile',
  'gradle',
  'properties',
  'tex',
  'srt',
  'vtt',
]);
const IMAGE_EXTENSIONS = new Set(['png', 'jpg', 'jpeg', 'webp', 'gif', 'bmp']);

export type FileKind = 'image' | 'pdf' | 'text';

/** What Hearth makes of a file, by its type and name (`null`: it can't attach it). */
export function kindOf(name: string, type: string): FileKind | null {
  const lower = name.toLowerCase();
  const dot = lower.lastIndexOf('.');
  const extension = dot >= 0 ? lower.slice(dot + 1) : '';
  if (type.startsWith('image/') || IMAGE_EXTENSIONS.has(extension)) {
    return type === 'image/svg+xml' || extension === 'svg' ? 'text' : 'image';
  }
  if (type === 'application/pdf' || extension === 'pdf') return 'pdf';
  if (
    type.startsWith('text/') ||
    /json|xml|javascript|yaml|toml|x-sh/.test(type) ||
    TEXT_EXTENSIONS.has(extension) ||
    // Makefile, LICENSE, Dockerfile…
    dot < 0
  ) {
    return 'text';
  }
  return null;
}

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/** Images already loaded (or just attached), by attachment id. */
export const imageCache = new Map<string, string>();

/** Reads one file. Throws an Error that says why it can't be attached. */
export async function readAttachment(file: File): Promise<NewAttachment> {
  const kind = kindOf(file.name, file.type);
  if (!kind) {
    throw new Error(`${file.name}: Hearth can attach text and code files, PDFs and images.`);
  }
  if (file.size > MAX_BYTES) throw new Error(`${file.name} is over 25 MB.`);
  const base = { name: file.name, size: file.size };
  if (kind === 'image') {
    return { ...base, kind: 'image', mime: file.type || 'image/png', data: await resized(file) };
  }
  if (kind === 'pdf') {
    return { ...base, kind: 'text', mime: 'application/pdf', text: await pdfText(file) };
  }
  const text = await file.text();
  if (text.includes('\u0000')) throw new Error(`${file.name} isn't a text file.`);
  return {
    ...base,
    kind: 'text',
    mime: file.type || 'text/plain',
    text: text.slice(0, MAX_TEXT_CHARS),
  };
}

async function resized(file: File): Promise<string> {
  let bitmap: ImageBitmap;
  try {
    bitmap = await createImageBitmap(file);
  } catch {
    throw new Error(`${file.name} couldn't be read as an image.`);
  }
  const scale = Math.min(1, IMAGE_SIDE / Math.max(bitmap.width, bitmap.height));
  const canvas = document.createElement('canvas');
  canvas.width = Math.max(1, Math.round(bitmap.width * scale));
  canvas.height = Math.max(1, Math.round(bitmap.height * scale));
  const context = canvas.getContext('2d');
  if (!context) throw new Error(`${file.name} couldn't be read as an image.`);
  context.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
  bitmap.close();
  // PNGs are often screenshots (sharp text, transparency): they stay PNG.
  return file.type === 'image/png'
    ? canvas.toDataURL('image/png')
    : canvas.toDataURL('image/jpeg', 0.9);
}

async function pdfText(file: File): Promise<string> {
  const pdfjs = await import('pdfjs-dist');
  const worker = await import('pdfjs-dist/build/pdf.worker.min.mjs?url');
  pdfjs.GlobalWorkerOptions.workerSrc = worker.default;
  const loading = pdfjs.getDocument({
    data: new Uint8Array(await file.arrayBuffer()),
    useSystemFonts: false,
    disableFontFace: true,
  });
  const pdf = await loading.promise;
  const pages: string[] = [];
  let chars = 0;
  try {
    for (let n = 1; n <= pdf.numPages && chars < MAX_TEXT_CHARS; n++) {
      const page = await pdf.getPage(n);
      const content = await page.getTextContent();
      let text = '';
      for (const item of content.items) {
        if (!('str' in item)) continue;
        text += item.str + (item.hasEOL ? '\n' : ' ');
      }
      text = text.replace(/[ \t]+\n/g, '\n').trim();
      pages.push(`[Page ${n}]\n${text}`);
      chars += text.length;
    }
  } finally {
    await loading.destroy();
  }
  if (chars === 0) {
    throw new Error(`${file.name} has no text to read (it may be scanned pages).`);
  }
  return pages.join('\n\n');
}
