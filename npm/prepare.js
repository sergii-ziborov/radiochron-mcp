'use strict';

const { chmodSync, copyFileSync, existsSync, mkdirSync, readFileSync, statSync } = require('node:fs');
const { createHash } = require('node:crypto');
const { join, resolve } = require('node:path');
const { spawnSync } = require('node:child_process');
const packageJson = require('../package.json');

const root = resolve(__dirname, '..');
const targets = [
  { key: 'win32-x64', env: 'RADIOCHRON_MCP_BINARY_WIN32_X64', file: 'radiochron.exe', windows: true, platform: 'windows', arch: 'x86_64' },
  { key: 'linux-x64', env: 'RADIOCHRON_MCP_BINARY_LINUX_X64', file: 'radiochron', windows: false, platform: 'linux', arch: 'x86_64' },
  { key: 'darwin-x64', env: 'RADIOCHRON_MCP_BINARY_DARWIN_X64', file: 'radiochron', windows: false, platform: 'macos', arch: 'x86_64' },
  { key: 'darwin-arm64', env: 'RADIOCHRON_MCP_BINARY_DARWIN_ARM64', file: 'radiochron', windows: false, platform: 'macos', arch: 'aarch64' }
];

function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, encoding: 'utf8', windowsHide: true });
  if (result.status !== 0) {
    const detail = (result.stderr || result.stdout || '').trim();
    throw new Error(`${command} ${args.join(' ')} failed${detail ? `: ${detail}` : ''}`);
  }
  return result.stdout.trim();
}

let expectedGitSha;
try {
  const dirty = run('git', ['status', '--porcelain', '--untracked-files=all']);
  if (dirty) throw new Error('working tree is not clean; commit the exact package sources first');
  expectedGitSha = run('git', ['rev-parse', 'HEAD']);
} catch (error) {
  console.error(`prepare: cannot establish source provenance: ${error.message}`);
  process.exit(1);
}

for (const target of targets) {
  const configured = process.env[target.env];
  if (!configured) {
    console.error(`prepare: ${target.env} must point to the verified ${target.key} server binary`);
    process.exit(1);
  }
  const source = resolve(configured);
  const buildInfoPath = `${source}.build-info.json`;
  const checksumPath = `${source}.sha256`;
  try {
    if (!existsSync(source) || !existsSync(buildInfoPath) || !existsSync(checksumPath)) {
      throw new Error('binary, build-info, or checksum sidecar is missing');
    }
    const buildInfo = JSON.parse(readFileSync(buildInfoPath, 'utf8'));
    const expectedChecksum = readFileSync(checksumPath, 'utf8').trim().split(/\s+/)[0].toLowerCase();
    const actualChecksum = createHash('sha256').update(readFileSync(source)).digest('hex');
    if (actualChecksum !== expectedChecksum) throw new Error('SHA-256 does not match the build sidecar');
    if (
      buildInfo.name !== 'radiochron' ||
      buildInfo.version !== packageJson.version ||
      buildInfo.git_sha !== expectedGitSha ||
      buildInfo.platform !== target.platform ||
      buildInfo.arch !== target.arch
    ) {
      throw new Error('binary identity, version, or source revision does not match this package');
    }
  } catch (error) {
    console.error(`prepare: cannot verify ${target.key}: ${error.message}`);
    process.exit(1);
  }

  const destinationDirectory = join(root, 'vendor', target.key);
  mkdirSync(destinationDirectory, { recursive: true });
  const destination = join(destinationDirectory, target.file);
  copyFileSync(source, destination);
  if (!target.windows) chmodSync(destination, 0o755);
  console.log(`prepare: bundled ${target.key} (${Math.round(statSync(source).size / 1024)} KB, ${expectedGitSha.slice(0, 12)})`);
}
