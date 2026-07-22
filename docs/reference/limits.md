# Input Size Limits

[Home](../../) > [Docs](../) > Limits

Structural indices store counters in narrow integer types where the memory
cost of widening would be significant. Since [#188](https://github.com/rust-works/succinctly/issues/188),
every such ceiling is validated at build time and fails loudly instead of
silently truncating.

## Enforced Ceilings

| Structure                        | Ceiling                        | On violation                              | Why not widen?                                          |
|----------------------------------|--------------------------------|-------------------------------------------|---------------------------------------------------------|
| `JsonIndex::build`/`from_parts`  | `u32::MAX` bytes (< 4 GiB)     | Panic (documented)                        | IB rank array is ~6.25% of input; u64 doubles it        |
| `YamlIndex::build`               | `u32::MAX` bytes (< 4 GiB)     | `Err(YamlError::InputTooLarge)`           | Text positions stored as u32 throughout the semi-index  |
| `DsvIndexLightweight::new`       | `u32::MAX` bytes (< 4 GiB)     | Panic (documented)                        | Two rank arrays are ~12.5% of input combined            |
| `BalancedParens` constructors    | `u32::MAX` bits                | Panic (documented)                        | `rank_l1` stores absolute cumulative rank as u32        |

The `BalancedParens` ceiling acts as a backstop for pathological JSON/YAML
that emits more BP bits than input bytes: such inputs can exceed the BP
ceiling before the byte ceiling, and the constructor assert catches them with
a clear message.

## Widened Counters (no ceiling)

| Structure                        | Change                          | Reason                                                       |
|----------------------------------|---------------------------------|--------------------------------------------------------------|
| `BalancedParens` L2 excess       | `i16` → `i32`                   | Nesting depth > 32767 is realistic; widening costs ~nothing  |
| `SelectIndex` sample entries     | `u32` → `u64`                   | > 2^32 set bits (~512 MB of ones) is within `huge-tests`     |

Nesting depth is therefore no longer bounded by the index (previously
~32,767); `BitVec`/`SelectIndex` support bitvectors past 2^32 set bits.

## Practical Notes

- The CLI tools (`sjq`, `syq`, DSV input) inherit these limits: a > 4 GiB
  input file fails with the corresponding error or panic message rather than
  producing wrong results.
- All ceilings are per-document. Multi-document YAML streams are bounded by
  the total input passed to a single `YamlIndex::build` call.
- The design decision (widen cheap counters, validate ceilings on hot
  arrays) is recorded in [#188](https://github.com/rust-works/succinctly/issues/188).

## See Also

- [BalancedParens](../architecture/balanced-parens.md) — rank directory and excess index internals
- [BitVec](../architecture/bitvec.md) — rank/select directories
- [Semi-indexing](../architecture/semi-indexing.md) — why indices track text positions
