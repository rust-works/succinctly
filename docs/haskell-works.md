# haskell-works Package Analysis

This document provides an analysis of the `hw-*` (haskell-works) package ecosystem.

## Overview

| Package | Version | Description | License |
|---------|---------|-------------|---------|
| [hw-aeson](https://github.com/haskell-works/hw-aeson) | 0.1.8.0 | Convenience functions for Aeson | BSD-3-Clause |
| [hw-bits](https://github.com/haskell-works/hw-bits) | 0.7.2.2 | Bit manipulation | BSD-3-Clause |
| [hw-dsv](https://github.com/haskell-works/hw-dsv) | 0.4.1.2 | Fast streaming DSV/CSV parser | BSD-3-Clause |
| [hw-hspec-hedgehog](https://github.com/haskell-works/hw-hspec-hedgehog) | 0.1.1.1 | Hspec-Hedgehog interoperability | BSD-3-Clause |
| [hw-json](https://github.com/haskell-works/hw-json) | 1.3.3.0 | Memory efficient JSON parser | BSD-3-Clause |
| [hw-json-simd](https://github.com/haskell-works/hw-json-simd) | 0.1.1.2 | SIMD-based JSON semi-indexer | BSD-3-Clause |
| [hw-mquery](https://github.com/haskell-works/hw-mquery) | 0.2.1.1 | Monadic query DSL | BSD-3-Clause |
| [hw-polysemy](https://github.com/haskell-works/hw-polysemy) | 0.3.1.2 | Opinionated polysemy library | Apache-2.0 |
| [hw-prelude](https://github.com/haskell-works/hw-prelude) | 0.0.4.1 | Opinionated prelude library | Apache-2.0 |
| [hw-prim](https://github.com/haskell-works/hw-prim) | 0.6.3.2 | Primitive functions and data types | BSD-3-Clause |
| [hw-simd](https://github.com/haskell-works/hw-simd) | 0.1.2.2 | SIMD library | BSD-3-Clause |
| [hw-string-parse](https://github.com/haskell-works/hw-string-parse) | 0.0.0.5 | String parser | BSD-3-Clause |
| [hw-xml](https://github.com/haskell-works/hw-xml) | 0.5.1.2 | XML parser (succinct data structures) | BSD-3-Clause |

## Dependency Graph

![haskell-works dependency graph](haskell-works.png)

## Inter-Project Dependencies

### Dependency Matrix

| Package | Depends On (hw-* only) |
|---------|------------------------|
| **hw-aeson** | *(none)* |
| **hw-bits** | hw-prim, hw-string-parse, hw-int, hw-hspec-hedgehog (test) |
| **hw-dsv** | hw-bits, hw-prim, hw-simd, hw-rankselect, hw-rankselect-base, hw-hspec-hedgehog (test) |
| **hw-hspec-hedgehog** | *(none)* |
| **hw-json** | hw-bits, hw-prim, hw-mquery, hw-balancedparens, hw-parser, hw-rankselect, hw-rankselect-base, hw-simd (x86_64), hw-json-simd (x86_64), hw-json-simple-cursor, hw-json-standard-cursor, hw-hspec-hedgehog (test) |
| **hw-json-simd** | hw-prim |
| **hw-mquery** | hw-hspec-hedgehog (test) |
| **hw-polysemy** | hw-prelude |
| **hw-prelude** | *(none)* |
| **hw-prim** | hw-hspec-hedgehog (test) |
| **hw-simd** | hw-bits, hw-prim, hw-rankselect, hw-rankselect-base, hw-hedgehog (test), hw-hspec-hedgehog (test) |
| **hw-string-parse** | *(none)* |
| **hw-xml** | hw-bits, hw-prim, hw-balancedparens, hw-parser, hw-rankselect, hw-rankselect-base, hw-hspec-hedgehog (test) |

### External hw-* Dependencies

These packages are referenced but are not part of this analysis:

| External Package | Used By |
|------------------|---------|
| **hw-rankselect** | hw-dsv, hw-json, hw-simd, hw-xml |
| **hw-rankselect-base** | hw-dsv, hw-json, hw-simd, hw-xml |
| **hw-balancedparens** | hw-json, hw-xml |
| **hw-parser** | hw-json, hw-xml |
| **hw-int** | hw-bits |
| **hw-hedgehog** | hw-simd (test) |
| **hw-json-simple-cursor** | hw-json |
| **hw-json-standard-cursor** | hw-json |
| **hw-ip** | hw-dsv (executable only) |

## Dependency Layers

### Layer 0 - Foundation (No hw-* dependencies)

- `hw-aeson` - Standalone Aeson utilities
- `hw-hspec-hedgehog` - Test framework bridge
- `hw-string-parse` - String parsing utilities
- `hw-prelude` - Opinionated prelude

### Layer 1 - Core (Minimal dependencies)

- `hw-prim` - Core primitives (uses hw-hspec-hedgehog for tests only)
- `hw-mquery` - Query DSL (uses hw-hspec-hedgehog for tests only)
- `hw-json-simd` - SIMD JSON indexer (depends on hw-prim)
- `hw-polysemy` - Polysemy effects (depends on hw-prelude)

### Layer 2 - Building Blocks

- `hw-bits` - Bit manipulation (hw-prim, hw-string-parse, hw-int)

### Layer 3 - SIMD/Performance Layer

- `hw-simd` - SIMD operations (hw-bits, hw-prim, hw-rankselect*)

### Layer 4 - Applications (High-level Parsers)

- `hw-dsv` - CSV/DSV parser (hw-bits, hw-prim, hw-simd, hw-rankselect*)
- `hw-json` - JSON parser (hw-bits, hw-prim, hw-mquery, hw-simd, hw-json-simd, hw-balancedparens*, etc.)
- `hw-xml` - XML parser (hw-bits, hw-prim, hw-balancedparens*, hw-rankselect*)

## GHC Compatibility

| Package | Tested GHC Versions |
|---------|---------------------|
| hw-dsv | 9.12.2, 9.10.2, 9.8.4, 9.6.7 |
| hw-prim | 9.12.1, 9.10.1, 9.8.4, 9.6.6, 9.4.8, 9.2.8, 9.0.2, 8.10.7, 8.8.4 |
| hw-json | 9.8.2, 9.6.6 |
| hw-polysemy | 9.10.1, 9.8.4, 9.6.6, 9.4.8 |
| hw-prelude | *(not specified)* |
| Others | Typically 9.2.2, 9.0.2, 8.10.7, 8.8.4, 8.6.5 |

## SIMD/Performance Features

| Package | SSE4.2 | AVX2 | BMI2 | C Sources |
|---------|--------|------|------|-----------|
| hw-bits | Yes | - | - | - |
| hw-dsv | Yes | Yes | Yes | - |
| hw-json | Yes | - | Yes | - |
| hw-json-simd | Yes | Yes | Yes | Yes (simd*.c) |
| hw-prim | Yes | - | - | - |
| hw-simd | Yes | Yes | Yes | Yes (simd_avx2.c, simd_sse2.c) |
| hw-string-parse | Yes | - | - | - |
| hw-xml | Yes | - | - | - |

**Note**: `hw-json-simd` and `hw-simd` are disabled on ARM/aarch64 architectures.

## Components Summary

| Package | Libraries | Executables | Test Suites | Benchmarks |
|---------|-----------|-------------|-------------|------------|
| hw-aeson | 1 | - | 2 | - |
| hw-bits | 1 | - | 2 | 1 |
| hw-dsv | 1 | 1 | 3 | 1 |
| hw-hspec-hedgehog | 1 | - | 2 | - |
| hw-json | 2 (lib + examples) | 1 | 2 | 1 |
| hw-json-simd | 1 | 1 | 2 | - |
| hw-mquery | 1 | 1 | 2 | - |
| hw-polysemy | 5 (sub-libraries) | - | 1 | - |
| hw-prelude | 1 | - | - | - |
| hw-prim | 1 | - | 2 | 1 |
| hw-simd | 1 | - | 2 | 1 |
| hw-string-parse | 1 | - | 2 | - |
| hw-xml | 1 | 1 | 2 | 1 |

### hw-polysemy Sub-libraries

The `hw-polysemy` package uses Cabal's public sub-library feature:

- `hw-polysemy:core` - Core polysemy effects
- `hw-polysemy:hedgehog` - Hedgehog integration
- `hw-polysemy:amazonka` - AWS Amazonka integration
- `hw-polysemy:testcontainers-localstack` - LocalStack test containers

## Testing Frameworks

- **hspec** + **hedgehog** - Most packages
- **hw-hspec-hedgehog** - Bridge between hspec and hedgehog
- **tasty** + **tasty-hedgehog** - hw-polysemy
- **doctest** - Most packages include doctest suites

## Key Observations

1. **Core foundation**: `hw-prim` is the most fundamental package, used by nearly all others

2. **Test infrastructure**: `hw-hspec-hedgehog` is widely used as a test dependency across the ecosystem

3. **Succinct data structure ecosystem**: Many packages depend on external `hw-rankselect`, `hw-rankselect-base`, and `hw-balancedparens` for succinct data structure operations

4. **Performance focus**: Heavy use of SIMD instruction sets (SSE4.2, AVX2, BMI2) with conditional compilation for optimal performance on x86_64

5. **Two package families**:
   - BSD-3-Clause licensed "data structure" packages (hw-prim, hw-bits, hw-json, etc.)
   - Apache-2.0 licensed "effect" packages (hw-polysemy, hw-prelude)

6. **Consistent authorship**: All packages maintained by John Ky (newhoggy@gmail.com)

7. **Modern Cabal**: Most packages use cabal-version 2.2; newer packages (hw-polysemy, hw-prelude) use 3.4 with advanced features like public sub-libraries
