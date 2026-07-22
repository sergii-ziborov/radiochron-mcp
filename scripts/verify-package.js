'use strict';

const { spawnSync } = require('node:child_process');
const { resolve } = require('node:path');

const windows = process.platform === 'win32';
const command = windows ? process.env.ComSpec || 'cmd.exe' : 'npm';
const args = windows
  ? ['/d', '/s', '/c', 'npm.cmd pack --dry-run --json --ignore-scripts']
  : ['pack', '--dry-run', '--json', '--ignore-scripts'];
const result = spawnSync(command, args, {
  cwd: resolve(__dirname, '..'),
  encoding: 'utf8',
  windowsHide: true
});

if (result.error) throw result.error;
if (result.status !== 0) {
  process.stderr.write(result.stderr || result.stdout || 'npm pack failed');
  process.exit(result.status ?? 1);
}

const [manifest] = JSON.parse(result.stdout);
const paths = manifest.files.map((file) => file.path.replaceAll('\\', '/'));
const forbidden = paths.filter((path) =>
  path.startsWith('target/') ||
  path.includes('/target/') ||
  path.startsWith('native/') ||
  path.startsWith('radiochron-js/') ||
  path.endsWith('.tgz')
);
if (forbidden.length > 0) {
  throw new Error(`package contains unrelated or build artifacts: ${forbidden.slice(0, 5).join(', ')}`);
}

const expectedBinaries = [
  'vendor/win32-x64/radiochron.exe',
  'vendor/linux-x64/radiochron',
  'vendor/linux-arm64/radiochron',
  'vendor/darwin-x64/radiochron',
  'vendor/darwin-arm64/radiochron'
];
for (const path of expectedBinaries) {
  if (!paths.includes(path)) throw new Error(`package is missing ${path}`);
}

if (manifest.entryCount > 50 || manifest.unpackedSize > 15_000_000) {
  throw new Error(`package is unexpectedly large: ${manifest.entryCount} files, ${manifest.unpackedSize} bytes unpacked`);
}

console.log(`verified ${manifest.id}: ${manifest.entryCount} files, ${manifest.size} bytes packed`);
