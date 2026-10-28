# Project status

This is an experimental, unofficial native integration. The target is AE2 19.2.18 behavior, but the port is incomplete. This page records the scope of the current checkpoint rather than certifying full compatibility.

## Reference versions

| Component | Pinned reference |
| --- | --- |
| Applied Energistics 2 | 19.2.18 for Minecraft 1.21.1, commit [`db17504a86128fdf3dae31f5fb7a112a646e0b93`](https://github.com/AppliedEnergistics/Applied-Energistics-2/commit/db17504a86128fdf3dae31f5fb7a112a646e0b93) |
| Horizon | Minecraft 26.3, commit [`7f8a6680a88f36a3c4a499f821862c447b72ce3f`](https://github.com/ludovic111/horizon/commit/7f8a6680a88f36a3c4a499f821862c447b72ce3f) |

Horizon and AE2 target different Minecraft versions here. The port maps selected AE2 behavior into Horizon; it does not load the upstream Java mod.

## What has been exercised

The current implementation includes paths for ME storage and fluids, power and channels, physical machines and upgrades, terminal and wireless interfaces, pattern encoding and crafting, links, spatial storage and the Guide. Individual paths have different levels of completeness.

The Stage 83 source checkpoint recorded 241 changed files in the Horizon tree and 26 files across five AE2 core crates. One historical `.orig` backup was intentionally omitted from the public export; the manifest records the 25 retained checkpoint files and that omission. A coordinated host gate reported **80 library tests and 245 client tests passing**, with **three library tests ignored** for resource prerequisites. These totals are a point-in-time test result, not a measure of overall feature coverage. The checkpoint source hashes and references are recorded in [SOURCE_MANIFEST.json](../SOURCE_MANIFEST.json).

A separate check of the exported source passed 128 standalone core tests (12, 38, 10, 7 and 61 across the five crates), and locked offline Cargo metadata resolution passed. This export check is separate from the earlier coordinated client/host gate and native-world runs.

Bounded world checks include:

- A finite-powered network with Drive cells, import/export, an external Storage Bus, a crafting CPU, a Pattern Provider and an assembler. It produced an additional 128 sticks from 16 logs through a real two-recipe chain; the resulting stock, player tools and an export barrel survived restart.
- Physical wireless terminal and Network Tool ownership and gesture checks, plus crafting terminal and encoded-pattern workflows.
- Selected ordinary-container Storage Bus access and sided slot limits, including brewing and furnace cases, and a separate multiple-bus scenario.
- Selected quantum, spatial, machine, emitter, memory-card, Guide and item-action scenarios.

The hall and terminal screenshots in the README come from a command-seeded copy of the integrated test world. The seed placed machines and supplied items for a focused integration check; it is not a survival playthrough or proof that every placement and progression path works.

## Known boundaries

- **Full AE2 parity is unfinished.** Many blocks, item interactions, menus, recipes, upgrades, edge cases and combinations still need implementation or verification.
- **No Java mod loader or modpack support is implied.** This is not a `.jar` or `.hmod`; ATM10 and other Java modpacks are unsupported.
- **Java-world migration has not been established.** Horizon targets a different Minecraft version and runtime. Do not assume an existing Java save can be opened safely.
- **The alternate in-frame path has a known UI gap.** A Pattern Provider can fail to open its menu because its local session/routing path is incomplete. The default server-thread path has broader testing.
- **Platform and multiplayer coverage are bounded.** The checkpoint's native-world testing was conducted on macOS with isolated profiles and selected local/peer scenarios. It does not certify every platform, dedicated server setup or multiplayer interaction.

Use a fresh world for experiments and preserve any existing saves. A passing targeted test demonstrates the named scenario only.

## Source and content

The modified Horizon workspace is in `ludovic111-horizon-7f8a668/`; the five `ae2-*-core` directories contain native supporting crates. Runtime content and Minecraft resources are generated locally, not checked in as game files. See the [README](../README.md) for preparation commands and [NOTICE.md](../NOTICE.md) for upstream attribution and asset terms.

