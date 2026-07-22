'use strict';

const assert = require('node:assert/strict');
const test = require('node:test');
const packageJson = require('../package.json');
const { resolveBinary, targetFor } = require('./bin');

test('package identity is owned by the MCP repository', () => {
  assert.equal(packageJson.name, 'radiochron-mcp');
  assert.equal(packageJson.mcpName, 'io.github.sergii-ziborov/radiochron');
  assert.equal(packageJson.repository.url, 'git+https://github.com/sergii-ziborov/radiochron-mcp.git');
});

test('native target selection covers Windows, Linux x64/ARM64 and both Mac architectures', () => {
  assert.deepEqual(targetFor('win32', 'x64'), { key: 'win32-x64', executable: 'radiochron.exe' });
  assert.equal(targetFor('linux', 'x64').key, 'linux-x64');
  assert.equal(targetFor('linux', 'arm64').key, 'linux-arm64');
  assert.equal(targetFor('darwin', 'x64').key, 'darwin-x64');
  assert.equal(targetFor('darwin', 'arm64').key, 'darwin-arm64');
  assert.match(resolveBinary({ platform: 'darwin', arch: 'arm64' }), /vendor[\\/]darwin-arm64[\\/]radiochron$/);
});

test('unsupported native targets fail closed', () => {
  assert.throws(() => targetFor('linux', 'ia32'), /unsupported platform/);
});
