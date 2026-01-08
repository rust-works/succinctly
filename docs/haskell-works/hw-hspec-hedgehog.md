# hw-hspec-hedgehog Analysis

## Project Overview

**Repository**: [haskell-works/hw-hspec-hedgehog](https://github.com/haskell-works/hw-hspec-hedgehog)
**Version**: 0.1.1.1
**Author**: John Ky (newhoggy@gmail.com)
**License**: BSD-3-Clause
**Category**: Test

### Purpose

`hw-hspec-hedgehog` is a lightweight Haskell library that provides interoperability between two popular testing frameworks:

1. **Hspec** - A testing framework for Haskell inspired by RSpec (Ruby) and HUnit, providing a behavior-driven development (BDD) style test structure with `describe` and `it` blocks.

2. **Hedgehog** - A property-based testing library for Haskell (similar to QuickCheck but with integrated shrinking and a more modern API).

The library bridges these two frameworks, allowing developers to embed Hedgehog property tests within Hspec test suites. This enables teams to leverage both the organizational benefits of Hspec's BDD-style structure and the powerful generative testing capabilities of Hedgehog.

### Core Problem Solved

When using Hspec as the primary test runner, developers may want to include Hedgehog property-based tests. However, Hedgehog's `Property` type doesn't directly integrate with Hspec's `Spec` monad. This library provides adapter functions (`require`, `requireProperty`, `requireTest`) that convert Hedgehog properties into HUnit assertions, which Hspec natively supports.

---

## Code Structure

### Project Layout

```
hw-hspec-hedgehog/
├── src/
│   └── HaskellWorks/
│       └── Hspec/
│           └── Hedgehog.hs      # Main module (only source file)
├── test/
│   ├── Spec.hs                  # Test entry point (hspec-discover)
│   └── HaskellWorks/
│       └── Hspec/
│           └── HedgehogSpec.hs  # Test specifications
├── doctest/
│   └── DoctestDriver.hs         # Doctest configuration
├── hw-hspec-hedgehog.cabal      # Package definition
└── cabal.project                # Cabal project file
```

### Dependencies

| Dependency     | Version Range     | Purpose                                      |
|----------------|-------------------|----------------------------------------------|
| base           | >= 4.11, < 5      | Haskell standard library                     |
| hedgehog       | >= 0.5, < 1.3     | Property-based testing framework             |
| hspec          | >= 2.6, < 3       | BDD-style test framework                     |
| HUnit          | >= 1.5, < 1.7     | Unit testing framework (provides Assertion)  |
| call-stack     | >= 0.2, < 0.5     | Call stack capture for error location        |
| transformers   | >= 0.5.6.2, < 0.7 | Monad transformers (for liftIO)              |

---

## Key Modules

### HaskellWorks.Hspec.Hedgehog (Main Module)

**Location**: `/src/HaskellWorks/Hspec/Hedgehog.hs`

This is the sole module in the library, exporting three functions:

#### Exported Functions

```haskell
require :: HasCallStack => Property -> Assertion
```
Runs a Hedgehog `Property` and converts the result to an HUnit `Assertion`. If the property fails, it throws an `HUnitFailure` exception with source location information.

```haskell
requireProperty :: HasCallStack => PropertyT IO () -> Assertion
requireProperty = require . property
```
A convenience function that takes a `PropertyT IO ()` action, wraps it in a `Property`, and runs it via `require`. Uses the default test count (typically 100).

```haskell
requireTest :: HasCallStack => PropertyT IO () -> Assertion
requireTest = require . withTests 1 . property
```
Similar to `requireProperty`, but runs only a single test iteration. Useful for one-off integration tests or tests where generation is deterministic.

#### Internal Helper

```haskell
location :: HasCallStack => Maybe SrcLoc
location = case reverse callStack of
  (_, loc) : _ -> Just loc
  []           -> Nothing
```
Extracts the source location from the call stack for error reporting.

---

## Haskell Language Features Leveraged

### 1. Language Extensions

| Extension          | Purpose                                                                 |
|--------------------|-------------------------------------------------------------------------|
| `FlexibleContexts` | Allows more general type class constraints (used with `HasCallStack`)   |
| `OverloadedStrings`| Enables string literals to be polymorphic (used for error messages)     |
| `CPP`              | Preprocessor for conditional compilation (in DoctestDriver.hs)          |

### 2. Type Classes and Constraints

- **`HasCallStack`**: A GHC-specific constraint that captures the call stack at runtime. When a function with this constraint is called, GHC records the source location, which can later be retrieved via `callStack`. This enables better error messages with precise locations.

- **`MonadIO`**: Used implicitly via `liftIO` to lift IO actions into the `Assertion` context.

### 3. Monadic Composition

The library leverages Haskell's monadic abstractions:

```haskell
require p = do
  result <- liftIO $ check p
  unless result $
    E.throwIO (HUnitFailure location $ Reason "...")
```

- Uses `do`-notation for sequencing effects
- `liftIO` to embed IO actions
- `unless` for conditional execution

### 4. Function Composition

The library makes elegant use of point-free style and function composition:

```haskell
requireProperty = require . property
requireTest = require . withTests 1 . property
```

### 5. Pattern Matching on Lists

```haskell
location = case reverse callStack of
  (_, loc) : _ -> Just loc
  []           -> Nothing
```

Matches on the reversed call stack to extract the bottom-most (original call site) location.

### 6. Maybe Type for Optional Values

The `location` function returns `Maybe SrcLoc`, gracefully handling the case where no call stack is available.

### 7. Exception Handling

Uses `Control.Exception.throwIO` to throw HUnit exceptions that integrate with the Hspec test runner's error handling.

---

## Computer Science Techniques Used

### 1. Adapter Pattern

The library implements the **Adapter Pattern** from object-oriented design, translated to a functional context. It adapts the Hedgehog testing interface to be compatible with the HUnit/Hspec interface:

```
Hedgehog Property
      │
      ▼ (adapt via require)
HUnit Assertion
      │
      ▼ (used within)
Hspec Spec
```

### 2. Property-Based Testing (QuickCheck-style)

While not implemented by this library, it facilitates the use of property-based testing, a technique where:

- Test inputs are randomly generated
- Properties (invariants) are specified rather than specific test cases
- Failures are automatically shrunk to minimal counterexamples
- Much larger input spaces are covered compared to example-based testing

### 3. Monadic Lifting

The use of `liftIO` demonstrates **monadic lifting**, a technique for embedding computations from one monad into another via a natural transformation.

### 4. Call Stack Introspection

The library uses runtime call stack introspection (via `HasCallStack`) for improved error reporting. This is a form of **reflection** - examining program structure at runtime.

### 5. Higher-Order Functions

Functions like `property`, `withTests`, and the composition operators are higher-order functions that take or return other functions.

### 6. Point-Free Programming

The definitions of `requireProperty` and `requireTest` use point-free style, expressing computation as function composition without explicitly naming arguments.

---

## Test Patterns and Usage

### Basic Usage Pattern

From the test suite (`HedgehogSpec.hs`):

```haskell
import HaskellWorks.Hspec.Hedgehog
import Hedgehog
import qualified Hedgehog.Gen as Gen
import qualified Hedgehog.Range as Range
import Test.Hspec

spec :: Spec
spec = describe "MyModule" $ do
  it "property description" $ do
    require $ property $ do
      x <- forAll (Gen.int Range.constantBounded)
      x === x  -- Property: any int equals itself
```

### Single Test Pattern

For deterministic or one-shot tests:

```haskell
it "integration test" $ requireTest $ do
  result <- liftIO someIOAction
  result === expectedValue
```

### Error Location Testing

The test suite verifies that error locations are correctly captured:

```haskell
it "captures correct source location on failure" $
  requireProperty failure `shouldThrow` \(HUnitFailure srcLocMaybe _) ->
    fmap srcLocModule srcLocMaybe == Just "HaskellWorks.Hspec.HedgehogSpec"
```

---

## Rust Porting Considerations

### Feasibility Assessment

**Overall Feasibility**: High (with caveats)

The library is small (approximately 30 lines of actual code) and conceptually simple. However, the Rust ecosystem has different testing paradigms that affect how such a port would be structured.

### Rust Ecosystem Equivalents

| Haskell Library | Rust Equivalent(s)              | Notes                              |
|-----------------|---------------------------------|------------------------------------|
| Hspec           | Built-in `#[test]`, `test-case` | Rust has native test support       |
| Hedgehog        | `proptest`, `quickcheck`        | Property-based testing crates      |
| HUnit           | Built-in `assert!` macros       | Native in Rust                     |
| call-stack      | `std::backtrace::Backtrace`     | Available since Rust 1.65 (stable) |

### Porting Strategy

In Rust, the equivalent functionality would likely be achieved through:

1. **Macro-based integration**: A procedural macro that wraps proptest/quickcheck properties to work seamlessly with the standard test harness.

2. **Trait-based adapters**: Implementing traits like `Testable` or custom traits that allow property tests to be run as standard tests.

```rust
// Conceptual example
use proptest::prelude::*;

#[macro_export]
macro_rules! require_property {
    ($prop:expr) => {
        proptest!(|()| $prop)
    };
}

// Or using a wrapper struct
pub struct PropertyTest<F>(pub F);

impl<F, E> PropertyTest<F>
where
    F: FnOnce() -> Result<(), E>,
    E: std::fmt::Debug,
{
    pub fn run(self) {
        (self.0)().expect("Property test failed");
    }
}
```

### Key Rust Features Applicable

1. **Procedural Macros**: For creating ergonomic test syntax similar to Hspec's `describe`/`it` blocks.

2. **The `?` Operator**: For propagating test failures through `Result<T, E>`.

3. **Traits for Polymorphism**: Instead of Haskell's type classes, use Rust traits to abstract over different property test types.

4. **Error Handling with `Result`**: Rust's `Result<T, E>` provides explicit error handling, analogous to exceptions in Haskell.

5. **Backtraces**: `std::backtrace::Backtrace` can capture call stacks for error location (similar to `HasCallStack`).

### Challenges in Rust Port

1. **Different Testing Paradigm**: Rust's test framework is built into the language. The need for a separate "adapter" library is reduced because crates like `proptest` already integrate with `cargo test`.

2. **No Monad Transformers**: Rust doesn't have an equivalent to Haskell's `PropertyT` monad transformer. `proptest` uses a different approach with the `TestRunner` type and `prop_compose!` macro.

3. **Compile-time vs Runtime**: Haskell's `hspec-discover` auto-discovers tests at runtime, while Rust discovers tests at compile time via the `#[test]` attribute.

4. **Exception Semantics**: Rust doesn't have exceptions in the same sense as Haskell. Panics serve a similar role but are used differently (typically for unrecoverable errors).

### Existing Rust Solutions

Before porting, consider that:

- **`proptest`** already integrates well with Rust's standard test framework
- **`test-case`** provides parameterized testing
- **`rstest`** offers fixtures and parametrized tests similar to pytest

A direct port may not be necessary. Instead, the value might be in creating a more opinionated testing framework that combines these capabilities with a BDD-style API.

### Minimal Rust Port Example

```rust
//! Hypothetical hw-hspec-hedgehog equivalent in Rust

use std::backtrace::Backtrace;
use proptest::test_runner::{TestRunner, Config};
use proptest::strategy::Strategy;

/// Run a property test, panicking on failure with location info
pub fn require<S, F>(strategy: S, test_fn: F)
where
    S: Strategy,
    F: Fn(S::Value) -> bool,
{
    let mut runner = TestRunner::new(Config::default());

    if let Err(e) = runner.run(&strategy, |v| {
        if test_fn(v) {
            Ok(())
        } else {
            Err(proptest::test_runner::TestCaseError::Fail("Property failed".into()))
        }
    }) {
        let bt = Backtrace::capture();
        panic!("Property test failed: {:?}\n{}", e, bt);
    }
}

/// Run a single test iteration (equivalent to requireTest)
pub fn require_test<S, F>(strategy: S, test_fn: F)
where
    S: Strategy,
    F: Fn(S::Value) -> bool,
{
    let config = Config {
        cases: 1,
        ..Config::default()
    };
    let mut runner = TestRunner::new(config);

    if let Err(e) = runner.run(&strategy, |v| {
        if test_fn(v) {
            Ok(())
        } else {
            Err(proptest::test_runner::TestCaseError::Fail("Property failed".into()))
        }
    }) {
        panic!("Test failed: {:?}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn test_reflexivity() {
        require(any::<i32>(), |x| x == x);
    }

    #[test]
    fn test_single_case() {
        require_test(Just(42), |x| x == 42);
    }
}
```

### Recommendations for Rust Implementation

1. **Evaluate Necessity**: Given that `proptest` and `quickcheck` already integrate with `cargo test`, determine if a separate library adds value.

2. **Focus on Ergonomics**: If proceeding, focus on providing a more ergonomic API, possibly through procedural macros.

3. **BDD-Style Wrapper**: Consider creating a BDD-style wrapper around the standard test framework, similar to what `speculoos` or `rspec`-style crates attempt.

4. **Leverage Existing Crates**: Build on top of `proptest` rather than reimplementing property-based testing from scratch.

---

## Summary

`hw-hspec-hedgehog` is a minimal but useful adapter library that bridges Haskell's Hspec and Hedgehog testing frameworks. It demonstrates elegant functional programming patterns including:

- The Adapter pattern for framework interoperability
- Monadic composition and lifting
- Point-free programming style
- Call stack introspection for improved error messages

For Rust porting, the core concept (adapting property tests to work with a unit test framework) is already largely addressed by existing crates like `proptest`. A Rust implementation would focus more on ergonomics and potentially a BDD-style API wrapper rather than the core bridging functionality.

The library serves as a good example of how Haskell's type system and monadic abstractions enable clean, composable testing infrastructure with minimal code.
