# Input Size Limits

[Home](../../) > [Docs](../) > Limits

Structural indices store counters in narrow integer types where the memory
cost of widening would be significant. Since [#188](https://github.com/rust-works/succinctly/issues/188),
every such ceiling is validated at build time and fails loudly instead of
silently truncating.

## Enforced Ceilings

| Structure                       | Ceiling                              | On violation                              | Why not widen?                                         |
|---------------------------------|--------------------------------------|-------------------------------------------|--------------------------------------------------------|
| `JsonIndex::build`/`from_parts` | `u32::MAX` interest bits and BP bits | Panic (documented); `try_build` is `None` | IB rank array is ~6.25% of input; u64 doubles it       |
| `YamlIndex::build`              | `u32::MAX` bytes (< 4 GiB)           | `Err(YamlError::InputTooLarge)`           | Text positions stored as u32 throughout the semi-index |
| `DsvIndexLightweight::new`      | `u32::MAX` bytes (< 4 GiB)           | Panic (documented)                        | Two rank arrays are ~12.5% of input combined           |
| `BalancedParens` constructors   | `u32::MAX` bits                      | Panic (documented)                        | `rank_l1` stores absolute cumulative rank as u32       |
| `LineIndex::build`              | `u32::MAX` bytes (< 4 GiB)           | Panic (documented)                        | Line starts are Elias-Fano encoded from u32 offsets    |

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

- The CLI tools (`syq`, `sjq-locate`, DSV input) inherit these limits: a > 4 GiB
  input file fails with the corresponding error or panic message rather than
  producing wrong results.
- `JsonIndex` holds a text of more than `u32::MAX` bytes when its interest
  bits and balanced parentheses are each at most `u32::MAX`: at most
  2,147,483,647 values, keys included, two parentheses each. A longer text of
  about two bytes a value (`[0,0,...]`) passes that
  ([#7537](https://github.com/rust-works/succinctly/issues/7537)): its counts,
  not its positions, are 32-bit. `sjq` indexes such a document through
  `JsonIndex::try_build`, and reads one past the counts without an index, as a
  tree ([#7425](https://github.com/rust-works/succinctly/issues/7425)); what it
  cannot read that way is listed in
  [jq limitations](../compliance/jq/limitations.md#a-document-of-4-gib-or-more-7425-7537).
  `JsonIndex::to_line_column` and `to_offset` scan such a text on each call,
  since `LineIndex` keeps its 4 GiB ceiling, and `sjq-locate` still refuses one.
- All ceilings are per-document. Multi-document YAML streams are bounded by
  the total input passed to a single `YamlIndex::build` call.
- The design decision (widen cheap counters, validate ceilings on hot
  arrays) is recorded in [#188](https://github.com/rust-works/succinctly/issues/188).

## See Also

- [BalancedParens](../architecture/balanced-parens.md) — rank directory and excess index internals
- [BitVec](../architecture/bitvec.md) — rank/select directories
- [Semi-indexing](../architecture/semi-indexing.md) — why indices track text positions
