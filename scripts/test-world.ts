/** The world-test entry point always uses isolated, hidden native clients. */
import { readFileSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { root, help } from './common';
const suites = [
  'provider-session', 'crafting-alternatives', 'crafting-remainders', 'crafting-container-reuse',
  'crafting-cpu-capacity', 'crafting-stale-inventory', 'export-bus-crafting-schedule',
  'inscriber-components', 'growth-accelerator-tagged', 'fe-p2p-readback', 'client-settings',
  'common-config-power', 'annihilation-sky-dust-config', 'spatial-anchor-random-config',
  'fe-partial-acceptance', 'p2p-configured-tax', 'stored-energy-boundary', 'crafting-owner-events',
  'dynamic-item-models', 'storage-fuzzy-fluid', 'name-press-components',
  'quartz-knife-name-component', 'inframe-terminal', 'terminal-search',
  'inframe-crafting-status', 'inframe-crafting-matrix', 'name-press-list-inheritance',
  'quantum-source-singularity', 'quantum-hopper-inventory', 'crafting-crafted-substitute',
  'spatial-source-component', 'ae2-component-commands', 'inframe-pattern-encoding',
  'memory-card-components', 'emitter-memory-components', 'wireless-source-components',
  'provider-memory-components', 'encoding-fast-clicks', 'source-component-crafting',
  'machine-memory-components', 'source-named-recipe', 'interface-source-capacity',
  'memory-card-rich-names', 'wrapped-workbench', 'facade-source-component', 'wireless-key-types',
  'crafting-large-substitutions', 'wrapped-player-inventory', 'crafting-substitute-container', 'quantum-three-frame-unload',
  'facade-materials', 'crafting-fluid-template', 'encoding-fake-gestures',
  'interface-stock-editor', 'anchor-grid-chunk-events', 'quantum-forced-ready', 'encoding-processing-amount', 'source-wrapped-crafting-input',
  'machine-menu-identity', 'encoding-fake-drag', 'fake-slot-drag-machines',
  'annihilation-container-custody', 'annihilation-container-overflow',
  'provider-condenser-fluid', 'storage-bus-condenser-fluid',
  'annihilation-machine-containers', 'annihilation-shulker-components', 'annihilation-beehive-components',
  'spatial-container-metadata', 'wrench-energy-components', 'wrench-cpu-escrow', 'inframe-machine-menus', 'inframe-wireless-access-point',
  'emitter-fake-gestures', 'client-container-component-wire', 'block-item-state-components',
  'crafting-break-large-escrow', 'source-nonunit-crafting', 'crafting-even-mixed-pushes', 'processing-mixed-ordered',
  'pattern-access-owner', 'inframe-bus-menu', 'inframe-emitter-menu',
  'encoding-fake-hotkeys', 'interface-fake-hotkeys', 'processing-large-provider-queue', 'processing-creative-interface', 'crafting-large-job-persistence', 'source-special-patterns',
];
// Explicitly selectable before-cases, not part of the default passing checkpoint.
const pending = new Set(['quantum-forced-ready']);
if (!help('Usage: bun run scripts/test-world.ts [--build METADATA.json] [--content DIRECTORY] [--in-frame] [SUITE ...]\nRuns only hidden, isolated fixtures, sequentially under a host rendering lease.\nSuites: ' + suites.join(', '))) {
  const args = process.argv.slice(2), selected: string[] = [], env = { ...process.env };
  let inFrame = false;
  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg === '--in-frame') inFrame = true;
    else if (arg === '--build') {
      if (!args[i + 1]) throw new Error('--build needs a metadata JSON path');
      const build = JSON.parse(readFileSync(resolve(args[++i]), 'utf8'));
      if (typeof build.binary !== 'string' || !/^[a-f0-9]{64}$/.test(build.binary_sha256)) throw new Error('Invalid selected build metadata');
      env.AE2_TEST_BINARY = resolve(build.binary); env.AE2_TEST_BINARY_SHA256 = build.binary_sha256;
    } else if (arg === '--content') {
      if (!args[i + 1]) throw new Error('--content needs a directory');
      env.AE2_CONTENT_DIR = resolve(args[++i]);
    } else if (suites.includes(arg)) selected.push(arg);
    else throw new Error(`Unknown fixture option or suite: ${arg}`);
  }
  const failed: string[] = [];
  for (const suite of selected.length ? selected : suites.filter(suite => !pending.has(suite))) {
    const child = Bun.spawn([process.execPath, 'run', join(root, 'tests/world', suite + '.ts'), ...(inFrame ? ['--in-frame'] : [])], { cwd: root, env, stdin: 'inherit', stdout: 'inherit', stderr: 'inherit' });
    const code = await child.exited;
    if (code !== 0) failed.push(suite);
  }
  if (failed.length) { console.error('Failed hidden-world suites:', failed.join(', ')); process.exitCode = 1; }
}
