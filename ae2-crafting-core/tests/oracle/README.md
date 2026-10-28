# Crafting-byte arithmetic oracle

`CraftingByteOracle.java` retains the pinned AE2 `ICraftingSimulationState.addStackBytes`, `AEPatternHelper.condenseStacks`, and the two `getNodeCount` methods, wrapping them in minimal key/state/tree stubs. The examples manually expand tree nodes and supply pattern-execution and remainder charges; they do not execute AE2's actual tree traversal, world, recipe decoder, or CPU. The method is LGPL-3.0-or-later; see `../../LICENSE` and the source attribution in the file.

Run with a JDK, placing compiled files outside the repository:

```sh
oracle_dir="$(mktemp -d)"
javac -d "$oracle_dir" ae2-crafting-core/tests/oracle/CraftingByteOracle.java
java -cp "$oracle_dir" CraftingByteOracle
```

Expected: `item_batch=208`, `mixed_fluid=28`, `alternatives=30`, `nested=29`, `cake=68`, and `manual_lazy_tree=4,5`. The corresponding Rust tests are in `ae2-crafting-core/src/lib.rs`.

## Actual AE2 tree classes against stubs

With Bun, a JDK, and the pinned AE2 source checkout, run:

```sh
AE2_REFERENCE=/path/to/ae2-reference bun ae2-crafting-core/tests/oracle/run-tree-oracle.ts
```

The runner verifies the source commit, copies `CraftingTreeNode.java` and `CraftingTreeProcess.java` into a temporary directory, removes their package/import/nullable annotations, and compiles the actual classes against `CraftingTreeOracle.java`'s minimal inventory, key, grid, pattern and request stubs. Expected results: `cake=68`, `reuse=34`, `reuse_4097=20509`, `output_is_also_remainder=21`, `crafted_substitute=29`, `crafted_substitute_batch=33`, `primary_pattern_prevents_fallback=1`, `different_amount_prevents_fallback=1`, `first_success=37`, `failed_first=61`, `pruned_ancestor=29`, `simulated_missing_iron=2`, `simulated_missing_gold=0`, `craft_less_amount=12`, `craft_less_bytes=34`, `substitute_split_bytes=28`, `substitute_split_10000_bytes=60016`, `differing_unit_split_10000_bytes=60016`, `condensed_substitute_split_10000_bytes=70016`, `returned_substitute_split_10000_bytes=70016`, `substitute_container_untracked_bytes=28`, `single_substitute_container_bytes=19`, `substitute_container_not_reused=1`, `primary_container_alternative_empty=29`, `fluid_only_rejects_bucket=1`, `fluid_first_bytes=19`, `fluid_first_bucket_preserved=1`, `fluid_first_water_consumed=0`, `fluid_first_bucket_fallback_bytes=19`, `fluid_first_mixed_batch_bytes=22`, `wide_4097_candidate_nodes_bytes=32787`, `many_4097_substitute_steps_bytes=12307`, `deep_256_bytes=2569`, and four sibling-contention cases reporting `1` missing. The CRAFT_LESS example runs the source's highest-bit attempt sequence against a shared actual tree.

This exercises source tree traversal and its quantity-limit behavior for the named cases. Its stubs do not reproduce Minecraft, AE2's full inventory/cache implementations, component equality, fuzzy substitutions, live providers, or CPU execution. Nothing from the source checkout or compiler output is copied into this repository.

The fluid-template runner extracts and compiles the actual pinned `AECraftingPattern.getItemOrFluidInput` method against test-only stubs:

```sh
AE2_REFERENCE=/path/to/ae2-reference bun ae2-crafting-core/tests/oracle/run-fluid-template-oracle.ts
```

It checks bucket and remainder eligibility, disabled fluid substitution, absent fluid and a fluid-capable milk bucket. These stubs do not reproduce Minecraft's full fluid-container registry or other mods' capabilities.

## Wrapped input and processing-provider source methods

```sh
AE2_REFERENCE=/path/to/ae2-reference bun ae2-crafting-core/tests/oracle/run-wrapped-input-oracle.ts
AE2_REFERENCE=/path/to/ae2-reference bun ae2-crafting-core/tests/oracle/run-provider-send-oracle.ts
```

Both runners verify the pinned revision, extract the named LGPL source methods into temporary Java files, and compile them against the minimal stubs in `SourceWrappedOracle.java` and `SourceProviderOracle.java`. The wrapped-input runner exercises `AECraftingPattern.getCraftingInputs`, `GenericStack.fromItemStack`/`unwrapItemStack` and `WrappedGenericStack.unwrapWhat`/`unwrapAmount`. It checks ordinary and component-bearing wrapped keys, wrapper outer and inner counts, empty/degenerate wrappers, a fluid key, and the nine-slot limit.

The provider runner exercises `AEProcessingPattern.pushInputsToExternalInventory`, its default interface method, `PatternProviderLogic.addToSendList`/`sendStacksOut`, and `CondenserMEStorage.insert`. It checks sparse mixed-key order, partial retries, a large amount in one queue entry, simulation, full-output rejection, and integer-scaled fluid power. These stubs are not Minecraft inventories or a running AE2 provider, so they do not establish host-world integration, chunk persistence, or actual machine capability behavior. The provider runner changes the extracted private methods' visibility only so the harness can invoke them; their bodies remain unchanged.
