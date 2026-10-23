import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';

const root = join(import.meta.dirname, '..', '..');
const EM = String.fromCharCode(0x2014);
const EN = String.fromCharCode(0x2013);
// Escaped forms in TS and Rust source, plus HTML entities.
const ESCAPED = new RegExp([String.raw`\\u\{?201[34]\}?`, '&m' + 'dash;', '&n' + 'dash;'].join('|'), 'i');

function walk(dir: string, accept: (rel: string) => boolean, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    if (['node_modules', 'target', 'dist', '.git', 'gen'].includes(name)) continue;
    const full = join(dir, name);
    const rel = relative(root, full).split('\\').join('/');
    if (rel.startsWith('docs/superpowers')) continue;
    if (statSync(full).isDirectory()) walk(full, accept, out);
    else if (accept(rel)) out.push(rel);
  }
  return out;
}

describe('copy rules', () => {
  it('no_em_dashes_in_ui_strings', () => {
    const files = [
      ...walk(join(root, 'src'), (r) => /\.(ts|tsx)$/.test(r)),
      ...walk(join(root, 'src-tauri', 'src'), (r) => r.endsWith('.rs')),
      'src-tauri/resources/catalog.json',
      ...['README.md', 'CONTRIBUTING.md', 'SECURITY.md', 'SUPPORT.md', 'CODE_OF_CONDUCT.md'],
      ...walk(join(root, 'docs'), (r) => /^docs\/[^/]+\.md$/.test(r)),
      ...walk(join(root, '.github'), (r) => /\.(md|ya?ml)$/.test(r)),
    ];
    expect(files.length).toBeGreaterThan(40);
    const offenders = files.filter((f) => {
      const text = readFileSync(join(root, f), 'utf8');
      return text.includes(EM) || text.includes(EN) || ESCAPED.test(text);
    });
    expect(offenders).toEqual([]);
  });
});
