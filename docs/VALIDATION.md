# Native validation checkpoint

The latest completed host gate recorded here is Wave 11 on macOS / Apple M3 Pro. The source target is AE2 19.2.18 at `db17504a86128fdf3dae31f5fb7a112a646e0b93`; the engine target is Horizon / Minecraft 26.3 at `7f8a6680a88f36a3c4a499f821862c447b72ce3f`.

This is bounded evidence, not certification of full AE2, Java-world or ATM10 compatibility. Later working-tree changes need their own gate. The parity ledgers distinguish implementation, named tests and remaining work.

## Reproducible inputs

- Client binary SHA-256: `3dd50add7863c9538c60dac638df0054302a446300f3d9df53905cce29da6283`.
- Frozen source archive SHA-256: `c4524acbb0dba7b6fa89808095c3f026af4b4bc50deedbf5de720bfae7049a86`.
- Rust 1.95.0, locked dependencies, `fast` profile, `horizon-client`, default features disabled, `ae2-lab` enabled.
- Runtime content generated locally from the pinned source and owned Minecraft resources. Prepared assets, saves and binaries are excluded from the repository.

## Automated checks

The AE2-filtered gate passed 106 library tests and 311 client tests; three library tests were ignored for resource prerequisites. The separately selected wrapped-stack gate passed seven tests, with one resource-dependent test ignored. The standalone crafting core passed 80 tests.

Actual pinned Java tree classes compiled against test-only host stubs agree with the native byte-accounting cases, including cake (68 bytes), reusable containers (34), crafted substitutes, source-greedy shortages, and four 10,000-push cases (60,016 or 70,016 bytes). These stubs do not represent a running Java game. Separate source oracles cover pathing on 128 synthetic graphs, stored-energy numeric boundaries, and seven actual QuantumCluster/Locatables readiness, pairing and unload/reload states.

## Physical world evidence

The retained fixtures exercise physical items and server/client transactions. Passing cases across the immutable development builds include:

- Source-only item/fluid cells and named patterns, Missing Content recovery, real consumption, output and restart; a named recipe with a stale encoded result selects and produces the current recipe's diamond.
- Mixed patterns, crafted substitutes, cake/container remainders, CPU capacity rejection, asynchronous export-bus calculation and card-removal invalidation, owner-only job events with a second peer, cancellation and restart.
- A 10,000-push confirmation preview at 60,016 bytes with all uncommitted physical stock preserved. It is a planner/custody test, not a 40,000-item execution benchmark.
- Source Memory Cards for machine/part settings, compatible generic imports, physical upgrade movement, Provider blank consumption, styled names, P2P frequency/LEDs and restart.
- Interface fluid target/stock limited to the source's registered 4,000 mB, real item stack maxima, four filled buckets accepted and the fifth retained. The earlier Wave 7 unlimited-fluid test was retracted after finding `useRegisteredCapacities`; it is not parity evidence.
- Explosion-produced source singularities, real Quantum chamber menus, hopper insertion/extraction callbacks and two-frame restart; source SpatialPlotInfo formatting, plot reuse, legacy native cells and retained emeralds.
- Fluid View Cell filtering with distinct component-bearing water entries; direct wireless link/energy/crafting components, key-type selections and restart.
- Hidden in-frame Provider, ME and Crafting terminals, physical matrix/refill/drag, encoding, search, clear-on-close, confirmation and job status. Other local menus remain separately bounded.
- A private copy of the existing 31-node native hall retains seven emeralds, sixteen iron and 256 sticks, plus power, through another restart. This checks a prior native save, not a Java save migration.

The earlier wrapped Workbench test used an AE2-screen creative clone that the source client prohibits. That pass was retracted. The corrected source-valid rapid pickup/ghost/custody/restart fixture passed on Wave 11; backend component-preserving conversion is separately unit-tested.

Some failed runs exposed implementation defects; others exposed stale client observations, an incorrect unordered-NBT assertion, or physically blocked rays. Corrected fixtures were rerun on the same immutable binary where applicable. Unresolved failures remain in the parity ledgers and are not counted as passing.

## Screen isolation

Every automated native client uses `--background`, an isolated game/profile directory, virtual input and a 30 FPS cap. A cross-process lease serializes rendering, including LAN peers. The harness asserts invisible, unfocused, windowed operation and no system-pointer capture before issuing test actions. Framebuffer captures do not read the real desktop.

Run `bun run scripts/test.ts --client` for the host gate and `bun run scripts/test-world.ts SUITE ...` for selected worlds. The world runner supports `--build METADATA.json` to pin binary SHA-256, `--content DIRECTORY`, and `--in-frame` for supported alternate-runtime cases. Local metadata, logs, transcripts and disposable saves stay under ignored `.local/`.

Wave 11 also passed actual three-frame first-registration pairing, unload without promoting the third frame, and original-pair reconnection. The old 128-depth, 4,096-node and CPU-step guards were replaced by an explicit continuation machine, interned nodes and fallible allocations; 256 source-Java layers, 4,097 nodes and 4,097 steps have differential cases. A real 4,097-step job and component-bearing escrow round-trip through NBT, using a ByteArray above the old modified-UTF-8 String boundary. Processing amount and Interface stock editors have physical Set/Back, per-item limits, restart and stale-owner checks.

On macOS, harness deadlines, control waits and shutdown timers use `mach_absolute_time`, matching awake-time progression; sleep pauses a budget and never causes an action to be resent. No display assertion is used. Lid closure can still suspend the machine.
