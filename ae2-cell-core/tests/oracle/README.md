# Storage fuzzy-search oracle

`java ae2-cell-core/tests/oracle/StorageFuzzyOracle.java` runs a small
standalone check using the bounds methods from AE2 19.2.18 at commit
`db17504a86128fdf3dae31f5fb7a112a646e0b93`. It needs a JDK with single-file
source execution and writes no classes into this repository.

The stub keys make the item damage and maximum damage explicit. The cases
cover a query whose maximum differs from the configured key, a matching range,
and a configured damage beyond the query maximum. The implementation is
attributed to `FuzzySearch.makeLowerBound` and `makeUpperBound`; this is not
a running AE2 installation. Fluid primary-key behavior follows
`KeyCounter.getSubIndex` and `VariantCounter.UnorderedVariantMap.findFuzzy`
and is separately exercised by the Rust cell tests and the prepared physical
world fixture.
