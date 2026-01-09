# hw-prelude Analysis

## Project Overview

**Repository**: https://github.com/haskell-works/hw-prelude
**Version**: 0.0.4.1
**License**: Apache-2.0
**Author**: John Ky

`hw-prelude` is an opinionated prelude library for Haskell. It serves as a curated re-export of commonly used types, functions, and type classes from the Haskell ecosystem, along with additional utility functions that fill gaps in the standard library. The library follows modern Haskell best practices with a focus on explicit imports and type-safe error handling.

### Design Philosophy

The library embraces several key design principles:

1. **No Implicit Prelude**: Uses `NoImplicitPrelude` extension, requiring explicit imports
2. **Text-First**: Prefers `Text` over `String` for text handling
3. **Type-Safe Error Handling**: Provides utilities for `Maybe`/`Either` handling without partial functions
4. **Modern Extensions**: Leverages GHC extensions like `OverloadedRecordDot`, `DataKinds`, and `BlockArguments`

---

## Code Structure and Key Modules

```
src/
└── HaskellWorks/
    ├── Prelude.hs              # Main prelude module with curated re-exports
    ├── Control/
    │   └── Monad.hs            # Monadic control flow utilities
    ├── Data/
    │   └── String.hs           # String processing re-exports
    ├── Either.hs               # Either utilities
    ├── Error.hs                # Error handling combinators
    ├── Error/
    │   ├── Types.hs            # Error type re-exports
    │   └── Types/
    │       ├── GenericError.hs # Generic error wrapper
    │       └── TimedOut.hs     # Timeout error type
    ├── FilePath.hs             # Cross-platform file path utilities
    ├── IO/
    │   ├── Network.hs          # Network module aggregator
    │   ├── Network/
    │   │   ├── NamedPipe.hs    # Windows named pipe support
    │   │   ├── Port.hs         # Port allocation utilities
    │   │   └── Socket.hs       # Socket utilities
    │   └── Process.hs          # Process management utilities
    ├── OS.hs                   # OS detection
    ├── Stack.hs                # Call stack introspection
    ├── String.hs               # ToString type class
    ├── ToText.hs               # ToText type class
    ├── Tuple.hs                # N-ary curry/uncurry functions
    └── Unsafe.hs               # Explicit unsafe operations
```

---

## Key Modules Analysis

### HaskellWorks.Prelude

The main prelude module providing a curated selection of types and functions:

**Primitive Types**:
- `Bool`, `Char`, `Maybe`, `Either`
- Numeric: `Int`, `Int8-64`, `Integer`, `Word`, `Word8-64`, `Float`, `Double`
- Text types: `String`, `Text`, `LazyText`, `ByteString`, `LazyByteString`

**Type Classes**:
- Standard: `Eq`, `Ord`, `Num`, `Show`, `Read`
- Functor hierarchy: `Functor`, `Applicative`, `Monad`, `MonadIO`, `MonadFail`, `MonadPlus`, `Alternative`
- Data structures: `Foldable`, `Traversable`, `Semigroup`, `Monoid`
- Advanced: `Contravariant`, `Divisible`, `Bifunctor`

**Notable Additions**:
- `tshow :: Show a => a -> Text` - Text version of `show`
- `LazyText` and `LazyByteString` type aliases for readability
- Re-exports of `ExceptT` for monadic error handling
- Call stack support: `HasCallStack`, `CallStack`, `withFrozenCallStack`
- Error handling combinators from `HaskellWorks.Error`

### HaskellWorks.Error

Provides elegant error handling combinators that avoid partial functions:

```haskell
-- Maybe handling
onNothing  :: Monad m => m a -> Maybe a -> m a
onNothingM :: Monad m => m a -> m (Maybe a) -> m a

-- Either handling
onLeft   :: Monad m => (e -> m a) -> Either e a -> m a
onLeftM  :: Monad m => (e -> m a) -> m (Either e a) -> m a
onLeft_  :: Monad m => m a -> Either e a -> m a
onLeftM_ :: Monad m => m a -> m (Either e a) -> m a

-- List disambiguation (exactly one element expected)
onMany   :: Monad m => (NonEmpty a -> m (Maybe a)) -> [a] -> m (Maybe a)
onManyM  :: Monad m => (NonEmpty a -> m (Maybe a)) -> m [a] -> m (Maybe a)
onMany_  :: Monad m => m (Maybe a) -> [a] -> m (Maybe a)
onManyM_ :: Monad m => m (Maybe a) -> m [a] -> m (Maybe a)
```

### HaskellWorks.Control.Monad

Provides bounded iteration combinators:

```haskell
-- Repeat up to n times until predicate returns True
repeatNUntilM_ :: Monad m => Int -> (Int -> m Bool) -> m ()

-- Repeat up to n times while predicate returns True
repeatNWhileM_ :: Monad m => Int -> (Int -> m Bool) -> m ()
```

### HaskellWorks.Error.Types

Defines common error types:

```haskell
newtype GenericError = GenericError { message :: Text }
newtype TimedOut = TimedOut { message :: Text }
```

### HaskellWorks.Either

```haskell
-- Try first Either, fall back to second if Left
orElse :: Either e a -> Either e a -> Either e a
```

### HaskellWorks.String and HaskellWorks.ToText

Type classes for string conversions:

```haskell
class ToString a where
  toString :: a -> String

class ToText a where
  toText :: a -> Text
```

Both provide instances for `String`, `Text`, and `LT.Text`. `ToText` also handles `TB.Builder`.

### HaskellWorks.Tuple

Generalized curry/uncurry for tuples up to 9 elements:

```haskell
uncurry3 :: (a -> b -> c -> z) -> (a, b, c) -> z
curry3   :: ((a, b, c) -> z) -> a -> b -> c -> z
-- ... through uncurry9/curry9
```

### HaskellWorks.IO.Network.Port

Network port utilities:

```haskell
randomPort      :: (MonadIO m, MonadFail m) => HostAddress -> m PortNumber
reserveRandomPort :: (MonadFail m, MonadResource m) => HostAddress -> m (ReleaseKey, PortNumber)
portInUse       :: MonadIO m => HostAddress -> PortNumber -> m Bool
```

### HaskellWorks.IO.Network.Socket

Socket operations:

```haskell
isPortOpen         :: Int -> IO Bool
canConnect         :: SockAddr -> IO ()
listenOn           :: Int -> IO Socket
doesSocketExist    :: FilePath -> IO Bool
allocateRandomPorts :: Int -> IO [Int]
```

### HaskellWorks.IO.Process

Process management with timeout support:

```haskell
maybeWaitForProcess    :: ProcessHandle -> IO (Maybe ExitCode)
waitSecondsForProcess  :: Int -> ProcessHandle -> IO (Either TimedOut (Maybe ExitCode))
```

### HaskellWorks.Stack

Call stack introspection:

```haskell
callerModuleName :: HasCallStack => String
```

### HaskellWorks.OS and HaskellWorks.FilePath

Cross-platform utilities:

```haskell
isWin32      :: Bool
exeSuffix    :: String       -- ".exe" on Windows, "" otherwise
addExeSuffix :: String -> String
```

### HaskellWorks.Unsafe

Explicit module for unsafe operations (must be explicitly imported):

```haskell
error     :: HasCallStack => [Char] -> a
undefined :: HasCallStack => a
```

---

## Haskell Language Features Leveraged

### GHC Extensions Used

| Extension               | Purpose                                                    |
|-------------------------|------------------------------------------------------------|
| `NoImplicitPrelude`     | Prevents automatic Prelude import, enabling custom prelude |
| `BlockArguments`        | Allows blocks as function arguments without `$`            |
| `DataKinds`             | Type-level programming with promoted data types            |
| `DeriveGeneric`         | Automatic Generic instance derivation                      |
| `DuplicateRecordFields` | Allow same field name in different records                 |
| `FlexibleContexts`      | More permissive type class constraints                     |
| `FlexibleInstances`     | More permissive instance declarations                      |
| `LambdaCase`            | `\case` syntax for pattern matching lambdas                |
| `MultiWayIf`            | Guard-like syntax in if expressions                        |
| `NoFieldSelectors`      | Disable auto-generated field accessor functions            |
| `OverloadedRecordDot`   | Use dot syntax for record field access                     |
| `OverloadedStrings`     | String literals can be any `IsString` type                 |
| `RankNTypes`            | Higher-rank polymorphism                                   |
| `ScopedTypeVariables`   | Type variables scope across function body                  |
| `TypeApplications`      | Explicit type application with `@`                         |
| `TypeOperators`         | Allow operators in type signatures                         |
| `TypeSynonymInstances`  | Allow type synonyms in instance heads                      |

### Key Type System Features

1. **Monad Transformers**: Uses `ExceptT` for composable error handling
2. **Type Classes**: Custom `ToString` and `ToText` for polymorphic string conversion
3. **Contravariant Functors**: Exports `Contravariant` and `Divisible` for consuming/partitioning patterns
4. **Resource Management**: Integrates with `resourcet` for bracket-pattern resource handling
5. **Call Stack Tracking**: Uses `HasCallStack` constraint for better error messages

---

## Computer Science Techniques and Patterns

### 1. Railway-Oriented Programming

The `onLeft`/`onNothing` family implements railway-oriented error handling, where errors are handled at decision points rather than through exceptions:

```haskell
-- Instead of throwing, provide fallback behavior
result <- onLeftM handleError $ someOperation
```

### 2. Resource Acquisition Is Initialization (RAII)

Port reservation uses `MonadResource` for deterministic resource cleanup:

```haskell
reserveRandomPort :: MonadResource m => HostAddress -> m (ReleaseKey, PortNumber)
```

### 3. Timeout Pattern with Racing

Process waiting implements timeout using `Async.race`:

```haskell
waitSecondsForProcess seconds hProcess =
  IO.race
    (IO.threadDelay (seconds * 1000000) >> pure (TimedOut "..."))
    (maybeWaitForProcess hProcess)
```

### 4. Bounded Iteration

`repeatNUntilM_` and `repeatNWhileM_` provide safe bounded loops, preventing infinite loops:

```haskell
repeatNUntilM_ n action = go 0
  where go i = when (i < n) $ do
          shouldTerminate <- action i
          unless shouldTerminate $ go (i + 1)
```

### 5. NonEmpty for Safety

`onMany` functions use `NonEmpty` to ensure at least one element exists when dealing with multiple results.

### 6. Explicit Unsafe Module

Separating `error` and `undefined` into `HaskellWorks.Unsafe` makes partial functions opt-in, encouraging safer code.

---

## Dependencies

| Package | Purpose |
|---------|---------|
| `aeson` | JSON serialization (likely for error types) |
| `async` | Concurrent programming, racing operations |
| `bytestring` | Efficient byte sequences |
| `contravariant` | Contravariant functor support |
| `directory` | File system operations |
| `filepath` | File path manipulation |
| `generic-lens` | Generic lens derivation |
| `microlens` | Lightweight lens library |
| `network` | Network socket operations |
| `process` | Process spawning and management |
| `resourcet` | Resource management monad transformer |
| `text` | Efficient Unicode text |
| `transformers` | Monad transformers (ExceptT) |
| `unliftio` | Lifted IO exceptions |
| `Win32` | Windows-specific APIs (conditional) |

---

## Rust Porting Considerations

### Feasibility Assessment

| Component | Feasibility | Notes |
|-----------|-------------|-------|
| **Prelude re-exports** | N/A | Rust's `std::prelude` serves this purpose |
| **Error combinators** | High | Useful and idiomatic in Rust |
| **ToString/ToText** | Medium | Rust has `ToString`, `Display`, `From` |
| **Tuple utilities** | Low | Macros exist; limited utility in Rust |
| **Network utilities** | Medium | Rust ecosystem has alternatives |
| **Process utilities** | Medium | Useful timeout patterns |
| **OS detection** | Low | Rust has `cfg!(target_os)` |

### High-Value Porting Candidates

#### 1. Error Handling Combinators

The `on_left`, `on_nothing` patterns translate well to Rust:

```rust
pub trait OptionExt<T> {
    fn on_none<F>(self, f: F) -> T where F: FnOnce() -> T;
    fn on_none_with<E, F>(self, f: F) -> Result<T, E> where F: FnOnce() -> E;
}

impl<T> OptionExt<T> for Option<T> {
    fn on_none<F>(self, f: F) -> T where F: FnOnce() -> T {
        self.unwrap_or_else(f)
    }

    fn on_none_with<E, F>(self, f: F) -> Result<T, E> where F: FnOnce() -> E {
        self.ok_or_else(f)
    }
}

pub trait ResultExt<T, E> {
    fn on_err<F>(self, f: F) -> T where F: FnOnce(E) -> T;
}

impl<T, E> ResultExt<T, E> for Result<T, E> {
    fn on_err<F>(self, f: F) -> T where F: FnOnce(E) -> T {
        self.unwrap_or_else(f)
    }
}
```

#### 2. Bounded Iteration

```rust
/// Repeat up to n times until predicate returns true
pub fn repeat_n_until<F>(n: usize, mut action: F)
where
    F: FnMut(usize) -> bool
{
    for i in 0..n {
        if action(i) {
            break;
        }
    }
}

/// Async version
pub async fn repeat_n_until_async<F, Fut>(n: usize, mut action: F)
where
    F: FnMut(usize) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for i in 0..n {
        if action(i).await {
            break;
        }
    }
}
```

#### 3. Either/Or Pattern

```rust
pub trait OrElse<T, E> {
    fn or_else_result(self, other: Result<T, E>) -> Result<T, E>;
}

impl<T, E> OrElse<T, E> for Result<T, E> {
    fn or_else_result(self, other: Result<T, E>) -> Result<T, E> {
        match self {
            Ok(v) => Ok(v),
            Err(_) => other,
        }
    }
}
```

#### 4. Process Timeout Pattern

```rust
use std::process::Child;
use std::time::Duration;
use tokio::time::timeout;

#[derive(Debug)]
pub struct TimedOut {
    pub message: String,
}

pub async fn wait_for_process_with_timeout(
    child: &mut Child,
    duration: Duration,
) -> Result<Option<std::process::ExitStatus>, TimedOut> {
    match timeout(duration, async { child.wait() }).await {
        Ok(result) => Ok(result.ok()),
        Err(_) => Err(TimedOut {
            message: "Timed out waiting for process".to_string(),
        }),
    }
}
```

#### 5. Random Port Allocation

```rust
use std::net::{TcpListener, SocketAddr};

pub fn random_port() -> std::io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

pub fn allocate_random_ports(n: usize) -> std::io::Result<Vec<u16>> {
    let listeners: Vec<_> = (0..n)
        .map(|_| TcpListener::bind("127.0.0.1:0"))
        .collect::<Result<_, _>>()?;

    let ports: Vec<_> = listeners
        .iter()
        .map(|l| l.local_addr().map(|a| a.port()))
        .collect::<Result<_, _>>()?;

    // Ports are released when listeners are dropped
    Ok(ports)
}

pub fn is_port_open(port: u16) -> bool {
    TcpListener::bind(format!("127.0.0.1:{}", port)).is_ok()
}
```

### What Would Not Port Well

1. **Type Class Hierarchy**: Rust traits don't form the same hierarchy (no `Applicative` as separate from `Monad`)
2. **Monad Transformers**: Rust uses `async/await` instead of transformer stacks
3. **Lazy Evaluation**: Rust is strict by default; lazy patterns require explicit `Lazy<T>` or closures
4. **NoImplicitPrelude Pattern**: Rust already requires explicit imports
5. **GHC Extensions**: Most are Haskell-specific type system features

### Recommended Rust Crate Structure

If porting, consider a utility crate with:

```
src/
├── lib.rs
├── error.rs      # on_left, on_nothing extension traits
├── iter.rs       # repeat_n_until, repeat_n_while
├── net/
│   ├── mod.rs
│   └── port.rs   # random_port, port_in_use
├── process.rs    # timeout-aware process waiting
└── result.rs     # Result extension traits
```

### Existing Rust Ecosystem Alternatives

| hw-prelude Feature | Rust Alternative |
|--------------------|------------------|
| `tshow` | `format!("{}", x)` / `.to_string()` |
| `ToString` | `std::string::ToString` |
| Error types | `thiserror`, `anyhow` |
| Process timeout | `tokio::time::timeout` |
| Random ports | `portpicker` crate |
| Lens-like | `derive-getters`, raw accessors |

---

## Summary

`hw-prelude` is a well-designed, focused prelude library that:

1. Curates essential Haskell types and functions
2. Provides elegant error handling combinators
3. Includes practical I/O utilities for networking and processes
4. Encourages explicit unsafe operation usage
5. Follows modern Haskell extension practices

For Rust porting, the most valuable components are:
- **Error handling extension traits** (`on_left`, `on_nothing` patterns)
- **Bounded iteration utilities**
- **Network port allocation helpers**
- **Process timeout patterns**

The library's design philosophy of explicit imports and safe error handling aligns well with Rust's own design principles, making conceptual porting straightforward even where exact API translation isn't possible.
