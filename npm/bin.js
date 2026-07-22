#!/usr/bin/env node
'use strict';

const { spawnSync } = require('node:child_process');
const { existsSync } = require('node:fs');
const { join } = require('node:path');

const TARGETS = {
  'win32-x64': 'radiochron.exe',
  'linux-x64': 'radiochron',
  'darwin-x64': 'radiochron',
  'darwin-arm64': 'radiochron'
};

function targetFor(platform = process.platform, arch = process.arch) {
  const key = `${platform}-${arch}`;
  const executable = TARGETS[key];
  if (!executable) {
    throw new Error(`unsupported platform ${key}; supported: ${Object.keys(TARGETS).join(', ')}`);
  }
  return { key, executable };
}

function resolveBinary(options = {}) {
  const target = targetFor(options.platform, options.arch);
  return join(__dirname, '..', 'vendor', target.key, target.executable);
}

function main() {
  let executable;
  try {
    executable = resolveBinary();
  } catch (error) {
    process.stderr.write(`radiochron-mcp: ${error.message}\n`);
    return 1;
  }

  if (!existsSync(executable)) {
    process.stderr.write(`radiochron-mcp: bundled native server is missing for ${process.platform}-${process.arch}; reinstall the package.\n`);
    return 1;
  }

  const result = spawnSync(executable, process.argv.slice(2), { stdio: 'inherit', windowsHide: true });
  if (result.error) {
    process.stderr.write(`radiochron-mcp: could not start native server: ${result.error.message}\n`);
    return 1;
  }
  return result.status === null ? 1 : result.status;
}

if (require.main === module) {
  process.exitCode = main();
}

module.exports = { main, resolveBinary, targetFor };
