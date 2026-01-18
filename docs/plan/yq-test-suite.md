# yq Test Suite Design

## Scope

### First Iteration (In Scope)

All core query language features including:
- Path expressions and navigation
- Array and map operations
- String and numeric operations
- Comparison and boolean operators
- Variables and reduce
- Format functions (@json, @base64, etc.)
- Basic YAML features (anchors, aliases, explode)
- Control flow (select, alternative, pipe)
- Constructors (arrays, objects)

### First Iteration (Out of Scope)

The following features are excluded from the first iteration but may be added later:

| Feature | Reason |
|---------|--------|
| `load()`, `load_str()`, `load_base64()` | File system access - security concern |
| `env()`, `strenv()`, `envsubst()` | Environment variable access - security concern |
| `eval()` | Dynamic expression evaluation - complexity |
| `line_comment`, `head_comment`, `foot_comment` | Requires YAML parser changes to track comments |
| `style` (get/set) | Requires YAML parser changes to track style |
| `tag` (set) | Requires YAML parser changes to track tags |

Note: `tag` (get), `kind`, `type` are IN scope as they can be derived from cursor state.

## Test Data Sources (Public Domain)

All test data should use strings from public domain sources:

1. **Project Gutenberg texts**: Character names, book titles, quotes
   - Pride and Prejudice (Jane Austen)
   - Moby Dick (Herman Melville)
   - Alice in Wonderland (Lewis Carroll)
   - The Adventures of Sherlock Holmes (Arthur Conan Doyle)

2. **Scientific data**:
   - Planet names, chemical elements
   - Mathematical constants

3. **Historical data**:
   - US Presidents (pre-1923)
   - Historical dates

## Test Case Coding Scheme

```
YQ-{CATEGORY}-{NUMBER}

Categories:
  TRV = Traverse/Path expressions
  ARR = Array operations
  MAP = Map/Object operations
  STR = String operations
  NUM = Numeric operations
  CMP = Comparison operators
  BOL = Boolean operators
  VAR = Variables and reduce
  FMT = Format functions
  YML = YAML-specific features
  CTL = Control flow
  CON = Constructors
  ERR = Error handling
```

## Test Cases

### YQ-TRV: Traverse/Path Expressions

| Code | Description | Input | Expression | Expected Output |
|------|-------------|-------|------------|-----------------|
| YQ-TRV-001 | Identity | `{name: Darcy}` | `.` | `{name: Darcy}` |
| YQ-TRV-002 | Simple field | `{name: Darcy}` | `.name` | `Darcy` |
| YQ-TRV-003 | Nested field | `{a: {b: Ahab}}` | `.a.b` | `Ahab` |
| YQ-TRV-004 | Missing field | `{a: 1}` | `.b` | `null` |
| YQ-TRV-005 | Bracket field | `{"Mr. Darcy": hero}` | `.["Mr. Darcy"]` | `hero` |
| YQ-TRV-006 | Bracket dots | `{"a.b": 1}` | `.["a.b"]` | `1` |
| YQ-TRV-007 | Array index 0 | `[Mercury, Venus, Earth]` | `.[0]` | `Mercury` |
| YQ-TRV-008 | Array index 1 | `[Mercury, Venus, Earth]` | `.[1]` | `Venus` |
| YQ-TRV-009 | Negative index | `[Mercury, Venus, Earth]` | `.[-1]` | `Earth` |
| YQ-TRV-010 | Array slice | `[1, 2, 3, 4, 5]` | `.[1:3]` | `[2, 3]` |
| YQ-TRV-011 | Slice from start | `[1, 2, 3, 4, 5]` | `.[:2]` | `[1, 2]` |
| YQ-TRV-012 | Slice to end | `[1, 2, 3, 4, 5]` | `.[3:]` | `[4, 5]` |
| YQ-TRV-013 | Negative slice | `[1, 2, 3, 4, 5]` | `.[1:-1]` | `[2, 3, 4]` |
| YQ-TRV-014 | Multiple indices | `[a, b, c, d]` | `.[0, 2]` | `a` then `c` |
| YQ-TRV-015 | Array splat | `[Alice, Bob]` | `.[]` | `Alice` then `Bob` |
| YQ-TRV-016 | Map splat | `{a: 1, b: 2}` | `.[]` | `1` then `2` |
| YQ-TRV-017 | Optional missing | `{a: 1}` | `.b?` | `null` |
| YQ-TRV-018 | Optional present | `{a: 1}` | `.a?` | `1` |
| YQ-TRV-019 | Nested array | `{x: [{n: Alice}]}` | `.x[0].n` | `Alice` |
| YQ-TRV-020 | Recursive descent | `{a: {b: 1}}` | `.. \| select(. == 1)` | `1` |
| YQ-TRV-021 | Wildcard prefix | `{cat: 1, car: 2}` | `.["ca*"]` | `1` then `2` |
| YQ-TRV-022 | Wildcard suffix | `{redcat: 1, bluecat: 2}` | `.["*cat"]` | `1` then `2` |
| YQ-TRV-023 | Pipe chain | `{a: {b: Holmes}}` | `.a \| .b` | `Holmes` |
| YQ-TRV-024 | Deep path | `{a: {b: {c: {d: 42}}}}` | `.a.b.c.d` | `42` |
| YQ-TRV-025 | Numeric map key | `{1: one, 2: two}` | `.[1]` | `one` |

### YQ-ARR: Array Operations

| Code | Description | Input | Expression | Expected Output |
|------|-------------|-------|------------|-----------------|
| YQ-ARR-001 | Length | `[a, b, c]` | `length` | `3` |
| YQ-ARR-002 | First | `[Alice, Bob, Carol]` | `first` | `Alice` |
| YQ-ARR-003 | Sort numbers | `[3, 1, 4, 1, 5]` | `sort` | `[1, 1, 3, 4, 5]` |
| YQ-ARR-004 | Sort strings | `[cat, apple, bat]` | `sort` | `[apple, bat, cat]` |
| YQ-ARR-005 | Sort by field | `[{n: Bob}, {n: Alice}]` | `sort_by(.n)` | `[{n: Alice}, {n: Bob}]` |
| YQ-ARR-006 | Reverse | `[1, 2, 3]` | `reverse` | `[3, 2, 1]` |
| YQ-ARR-007 | Unique | `[1, 2, 2, 3, 1]` | `unique` | `[1, 2, 3]` |
| YQ-ARR-008 | Unique by | `[{n: a, v: 1}, {n: b}, {n: a, v: 2}]` | `unique_by(.n)` | `[{n: a, v: 1}, {n: b}]` |
| YQ-ARR-009 | Flatten | `[[1], [2, [3]]]` | `flatten` | `[1, 2, 3]` |
| YQ-ARR-010 | Flatten 1 | `[[1], [2, [3]]]` | `flatten(1)` | `[1, 2, [3]]` |
| YQ-ARR-011 | Group by | `[{t: a}, {t: b}, {t: a}]` | `group_by(.t)` | `[[{t: a}, {t: a}], [{t: b}]]` |
| YQ-ARR-012 | Min | `[5, 2, 8, 1]` | `min` | `1` |
| YQ-ARR-013 | Max | `[5, 2, 8, 1]` | `max` | `8` |
| YQ-ARR-014 | Add numbers | `[1, 2, 3]` | `add` | `6` |
| YQ-ARR-015 | Add strings | `[a, b, c]` | `add` | `abc` |
| YQ-ARR-016 | Any true | `[false, true, false]` | `any` | `true` |
| YQ-ARR-017 | Any false | `[false, false]` | `any` | `false` |
| YQ-ARR-018 | All true | `[true, true]` | `all` | `true` |
| YQ-ARR-019 | All false | `[true, false]` | `all` | `false` |
| YQ-ARR-020 | Any condition | `[1, 2, 3]` | `any_c(. > 2)` | `true` |
| YQ-ARR-021 | All condition | `[1, 2, 3]` | `all_c(. > 0)` | `true` |
| YQ-ARR-022 | Map | `[1, 2, 3]` | `map(. * 2)` | `[2, 4, 6]` |
| YQ-ARR-023 | Keys | `[a, b, c]` | `keys` | `[0, 1, 2]` |
| YQ-ARR-024 | Contains | `[cat, dog]` | `contains(["cat"])` | `true` |
| YQ-ARR-025 | Concat | `{a: [1], b: [2]}` | `.a + .b` | `[1, 2]` |
| YQ-ARR-026 | Shuffle | `[1, 2, 3, 4]` | `shuffle \| length` | `4` |
| YQ-ARR-027 | Empty min | `[]` | `min` | `null` |
| YQ-ARR-028 | Empty max | `[]` | `max` | `null` |

### YQ-MAP: Map/Object Operations

| Code | Description | Input | Expression | Expected Output |
|------|-------------|-------|------------|-----------------|
| YQ-MAP-001 | Length | `{a: 1, b: 2}` | `length` | `2` |
| YQ-MAP-002 | Keys | `{b: 2, a: 1}` | `keys` | `[b, a]` |
| YQ-MAP-003 | Values | `{a: 1, b: 2}` | `.[]` | `1` then `2` |
| YQ-MAP-004 | Has key true | `{name: Holmes}` | `has("name")` | `true` |
| YQ-MAP-005 | Has key false | `{name: Holmes}` | `has("age")` | `false` |
| YQ-MAP-006 | To entries | `{a: 1}` | `to_entries` | `[{key: a, value: 1}]` |
| YQ-MAP-007 | From entries | `[{key: a, value: 1}]` | `from_entries` | `{a: 1}` |
| YQ-MAP-008 | With entries | `{a: 1}` | `with_entries(.value \|= . * 2)` | `{a: 2}` |
| YQ-MAP-009 | Pick | `{a: 1, b: 2, c: 3}` | `pick(["a", "c"])` | `{a: 1, c: 3}` |
| YQ-MAP-010 | Omit | `{a: 1, b: 2, c: 3}` | `omit(["b"])` | `{a: 1, c: 3}` |
| YQ-MAP-011 | Sort keys | `{c: 3, a: 1, b: 2}` | `sort_keys(.)` | `{a: 1, b: 2, c: 3}` |
| YQ-MAP-012 | Map values | `{a: 1, b: 2}` | `map_values(. + 10)` | `{a: 11, b: 12}` |
| YQ-MAP-013 | Shallow merge | `{a: {x: 1}}` | `. + {b: 2}` | `{a: {x: 1}, b: 2}` |
| YQ-MAP-014 | Deep merge | `{a: {x: 1}}` | `. * {a: {y: 2}}` | `{a: {x: 1, y: 2}}` |
| YQ-MAP-015 | Merge override | `{a: 1}` | `. * {a: 2}` | `{a: 2}` |
| YQ-MAP-016 | Key operator | `{mykey: 1}` | `.mykey \| key` | `mykey` |
| YQ-MAP-017 | Pivot | `[{a: 1, b: x}, {a: 2, b: y}]` | `pivot` | `{a: [1, 2], b: [x, y]}` |
| YQ-MAP-018 | Contains map | `{a: 1, b: 2}` | `contains({a: 1})` | `true` |
| YQ-MAP-019 | Delete key | `{a: 1, b: 2}` | `del(.a)` | `{b: 2}` |
| YQ-MAP-020 | Merge append | `{a: [1]}` | `. *+ {a: [2]}` | `{a: [1, 2]}` |

### YQ-STR: String Operations

| Code | Description | Input | Expression | Expected Output |
|------|-------------|-------|------------|-----------------|
| YQ-STR-001 | Length | `"Elizabeth"` | `length` | `9` |
| YQ-STR-002 | Upcase | `"holmes"` | `upcase` | `HOLMES` |
| YQ-STR-003 | Downcase | `"WATSON"` | `downcase` | `watson` |
| YQ-STR-004 | Trim | `"  Darcy  "` | `trim` | `Darcy` |
| YQ-STR-005 | Split | `"a,b,c"` | `split(",")` | `[a, b, c]` |
| YQ-STR-006 | Join | `[a, b, c]` | `join("-")` | `a-b-c` |
| YQ-STR-007 | Test true | `"hello123"` | `test("[0-9]+")` | `true` |
| YQ-STR-008 | Test false | `"hello"` | `test("[0-9]+")` | `false` |
| YQ-STR-009 | Match | `"abc123"` | `match("[0-9]+")` | `{string: "123", offset: 3, length: 3, captures: []}` |
| YQ-STR-010 | Capture | `"abc-123"` | `capture("(?P<a>[a-z]+)-(?P<n>[0-9]+)")` | `{a: abc, n: "123"}` |
| YQ-STR-011 | Sub | `"hello world"` | `sub("world"; "universe")` | `hello universe` |
| YQ-STR-012 | Sub global | `"aaa"` | `sub("a"; "b"; "g")` | `bbb` |
| YQ-STR-013 | Concat | `{a: "Mr.", b: "Darcy"}` | `.a + " " + .b` | `Mr. Darcy` |
| YQ-STR-014 | Contains | `"Elizabeth Bennet"` | `contains("Bennet")` | `true` |
| YQ-STR-015 | To string | `42` | `to_string` | `"42"` |
| YQ-STR-016 | Repeat | `"ab"` | `. * 3` | `ababab` |
| YQ-STR-017 | Case insens test | `"HELLO"` | `test("(?i)hello")` | `true` |
| YQ-STR-018 | Empty split | `"abc"` | `split("")` | `[a, b, c]` |
| YQ-STR-019 | Unicode upcase | `"café"` | `upcase` | `CAFÉ` |
| YQ-STR-020 | Ltrim | `"  hello"` | `ltrim` | `hello` |

### YQ-NUM: Numeric Operations

| Code | Description | Input | Expression | Expected Output |
|------|-------------|-------|------------|-----------------|
| YQ-NUM-001 | Add | `{a: 5, b: 3}` | `.a + .b` | `8` |
| YQ-NUM-002 | Subtract | `{a: 10, b: 3}` | `.a - .b` | `7` |
| YQ-NUM-003 | Multiply | `{a: 4, b: 5}` | `.a * .b` | `20` |
| YQ-NUM-004 | Divide | `{a: 20, b: 4}` | `.a / .b` | `5` |
| YQ-NUM-005 | Modulo | `{a: 17, b: 5}` | `.a % .b` | `2` |
| YQ-NUM-006 | Float add | `{a: 3.14, b: 2.86}` | `.a + .b` | `6` |
| YQ-NUM-007 | Negative | `5` | `-5` | `-5` |
| YQ-NUM-008 | To number | `"42"` | `to_number` | `42` |
| YQ-NUM-009 | Float to num | `"3.14"` | `to_number` | `3.14` |
| YQ-NUM-010 | Divide float | `{a: 7, b: 2}` | `.a / .b` | `3.5` |
| YQ-NUM-011 | Add assign | `{a: 5}` | `.a += 3` | `{a: 8}` |
| YQ-NUM-012 | Multiply assign | `{a: 5}` | `.a *= 2` | `{a: 10}` |

### YQ-CMP: Comparison Operators

| Code | Description | Input | Expression | Expected Output |
|------|-------------|-------|------------|-----------------|
| YQ-CMP-001 | Equals true | `{a: 1, b: 1}` | `.a == .b` | `true` |
| YQ-CMP-002 | Equals false | `{a: 1, b: 2}` | `.a == .b` | `false` |
| YQ-CMP-003 | Not equals | `{a: 1, b: 2}` | `.a != .b` | `true` |
| YQ-CMP-004 | Greater than | `{a: 5, b: 3}` | `.a > .b` | `true` |
| YQ-CMP-005 | Greater equal | `{a: 5, b: 5}` | `.a >= .b` | `true` |
| YQ-CMP-006 | Less than | `{a: 3, b: 5}` | `.a < .b` | `true` |
| YQ-CMP-007 | Less equal | `{a: 5, b: 5}` | `.a <= .b` | `true` |
| YQ-CMP-008 | String compare | `{a: "zoo", b: "apple"}` | `.a > .b` | `true` |
| YQ-CMP-009 | Null equals | null input | `null == null` | `true` |
| YQ-CMP-010 | Null greater | null input | `null > null` | `false` |
| YQ-CMP-011 | Wildcard eq | `"hello"` | `. == "hel*"` | `true` |
| YQ-CMP-012 | Wildcard suffix | `"hello"` | `. == "*llo"` | `true` |
| YQ-CMP-013 | Array equals | `{a: [1,2], b: [1,2]}` | `.a == .b` | `true` |
| YQ-CMP-014 | Map equals | `{a: {x: 1}, b: {x: 1}}` | `.a == .b` | `true` |

### YQ-BOL: Boolean Operators

| Code | Description | Input | Expression | Expected Output |
|------|-------------|-------|------------|-----------------|
| YQ-BOL-001 | And true | null | `true and true` | `true` |
| YQ-BOL-002 | And false | null | `true and false` | `false` |
| YQ-BOL-003 | Or true | null | `false or true` | `true` |
| YQ-BOL-004 | Or false | null | `false or false` | `false` |
| YQ-BOL-005 | Not true | `true` | `not` | `false` |
| YQ-BOL-006 | Not false | `false` | `not` | `true` |
| YQ-BOL-007 | Compound | null | `(true and false) or true` | `true` |
| YQ-BOL-008 | Null is falsy | `null` | `. and true` | `false` |
| YQ-BOL-009 | Empty str truthy | `""` | `. and true` | `true` |
| YQ-BOL-010 | Zero truthy | `0` | `. and true` | `true` |

### YQ-VAR: Variables and Reduce

| Code | Description | Input | Expression | Expected Output |
|------|-------------|-------|------------|-----------------|
| YQ-VAR-001 | Simple var | `{a: 5}` | `.a as $x \| $x` | `5` |
| YQ-VAR-002 | Two vars | `{a: 5, b: 3}` | `.a as $x \| .b as $y \| $x + $y` | `8` |
| YQ-VAR-003 | Var in path | `{key: "a", a: 1}` | `.key as $k \| .[$k]` | `1` |
| YQ-VAR-004 | Reduce sum | `[1, 2, 3, 4]` | `.[] as $i ireduce (0; . + $i)` | `10` |
| YQ-VAR-005 | Reduce obj | `[{k: a, v: 1}, {k: b, v: 2}]` | `.[] as $i ireduce ({}; .[$i.k] = $i.v)` | `{a: 1, b: 2}` |
| YQ-VAR-006 | Reduce concat | `[a, b, c]` | `.[] as $i ireduce (""; . + $i)` | `abc` |
| YQ-VAR-007 | Multi-value var | `[1, 2]` | `.[] as $x \| $x * 2` | `2` then `4` |

### YQ-FMT: Format Functions

| Code | Description | Input | Expression | Expected Output |
|------|-------------|-------|------------|-----------------|
| YQ-FMT-001 | JSON encode | `{a: 1}` | `@json` | `{"a":1}` |
| YQ-FMT-002 | Base64 encode | `"hello"` | `@base64` | `aGVsbG8=` |
| YQ-FMT-003 | Base64 decode | `"aGVsbG8="` | `@base64d` | `hello` |
| YQ-FMT-004 | URI encode | `"hello world"` | `@uri` | `hello+world` |
| YQ-FMT-005 | Shell quote | `"hello world"` | `@sh` | `'hello world'` |
| YQ-FMT-006 | CSV array | `[a, b, c]` | `@csv` | `a,b,c` |
| YQ-FMT-007 | TSV array | `[a, b, c]` | `@tsv` | `a\tb\tc` |

### YQ-YML: YAML-Specific Features

| Code | Description | Input | Expression | Expected Output |
|------|-------------|-------|------------|-----------------|
| YQ-YML-001 | Get anchor | file with `&anc` | `.x \| anchor` | `anc` |
| YQ-YML-002 | Follow alias | file with alias | `.alias_field` | resolved value |
| YQ-YML-003 | Explode | file with aliases | `explode(.)` | expanded doc |
| YQ-YML-004 | Line comment | `{a: 1 # comment}` | `.a \| line_comment` | `comment` |
| YQ-YML-005 | Set style | `hello` | `. style="double"` | `"hello"` |
| YQ-YML-006 | Flow style | `{a: 1, b: 2}` | `. style="flow"` | `{a: 1, b: 2}` |
| YQ-YML-007 | Get tag | `42` | `tag` | `!!int` |
| YQ-YML-008 | Get kind | `{a: 1}` | `kind` | `map` |
| YQ-YML-009 | Kind scalar | `"hello"` | `kind` | `scalar` |
| YQ-YML-010 | Kind seq | `[1, 2]` | `kind` | `seq` |
| YQ-YML-011 | Doc index | multi-doc | `document_index` | `0` or `1` etc |
| YQ-YML-012 | Split doc | `[{a: 1}, {b: 2}]` | `.[] \| split_doc` | `{a: 1}` then `---` then `{b: 2}` |

### YQ-CTL: Control Flow

| Code | Description | Input | Expression | Expected Output |
|------|-------------|-------|------------|-----------------|
| YQ-CTL-001 | Select true | `[1, 2, 3]` | `.[] \| select(. > 1)` | `2` then `3` |
| YQ-CTL-002 | Select none | `[1, 2, 3]` | `.[] \| select(. > 10)` | (empty) |
| YQ-CTL-003 | Alternative null | `{a: null}` | `.a // "default"` | `default` |
| YQ-CTL-004 | Alternative value | `{a: "value"}` | `.a // "default"` | `value` |
| YQ-CTL-005 | Alt chain | `{a: null, b: null}` | `.a // .b // "end"` | `end` |
| YQ-CTL-006 | With operator | `{a: {b: 1}}` | `with(.a; .c = 2)` | `{a: {b: 1, c: 2}}` |
| YQ-CTL-007 | Eval | `{expr: ".a", a: 1}` | `eval(.expr)` | `1` |
| YQ-CTL-008 | Union | `{a: 1, b: 2}` | `.a, .b` | `1` then `2` |
| YQ-CTL-009 | Empty filter | `{a: 1}` | `.b // empty` | (empty) |
| YQ-CTL-010 | Nested select | `{items: [{ok: true}, {ok: false}]}` | `.items[] \| select(.ok)` | `{ok: true}` |

### YQ-CON: Constructors

| Code | Description | Input | Expression | Expected Output |
|------|-------------|-------|------------|-----------------|
| YQ-CON-001 | Empty array | null | `[]` | `[]` |
| YQ-CON-002 | Array literal | null | `[1, 2, 3]` | `[1, 2, 3]` |
| YQ-CON-003 | Array from expr | `{a: 1, b: 2}` | `[.a, .b]` | `[1, 2]` |
| YQ-CON-004 | Collect | `{a: 1, b: 2}` | `[.[]]` | `[1, 2]` |
| YQ-CON-005 | Empty object | null | `{}` | `{}` |
| YQ-CON-006 | Object literal | null | `{"a": 1}` | `{a: 1}` |
| YQ-CON-007 | Object from expr | `{name: Alice}` | `{"greeting": "hi", "who": .name}` | `{greeting: hi, who: Alice}` |
| YQ-CON-008 | Dynamic key | `{k: mykey, v: myval}` | `{(.k): .v}` | `{mykey: myval}` |
| YQ-CON-009 | Shorthand | `{name: Bob, age: 30}` | `{name, age}` | `{name: Bob, age: 30}` |

### YQ-ERR: Error Handling

| Code | Description | Input | Expression | Expected |
|------|-------------|-------|------------|----------|
| YQ-ERR-001 | Missing field | `{a: 1}` | `.b.c` | `null` (no error) |
| YQ-ERR-002 | Wrong type index | `"hello"` | `.[0]` | error |
| YQ-ERR-003 | Type mismatch add | `{a: "x", b: 1}` | `.a + .b` | error |
| YQ-ERR-004 | Optional suppresses | `"hello"` | `.[0]?` | `null` |
| YQ-ERR-005 | Invalid regex | `"hello"` | `test("[")` | error |

## Test Implementation Strategy

### Phase 1: Parser Tests
Test that all expressions parse correctly without errors.

### Phase 2: Evaluation Tests
Test that parsed expressions evaluate to correct results.

### Phase 3: Output Format Tests
Test that results format correctly as YAML/JSON.

### Phase 4: Conformance Tests
Run against real yq tool and compare outputs byte-for-byte.

## Test Data Files

### gutenberg_names.yaml
```yaml
novels:
  pride_and_prejudice:
    characters:
      - Elizabeth Bennet
      - Mr. Darcy
      - Jane Bennet
      - Mr. Bingley
  moby_dick:
    characters:
      - Captain Ahab
      - Ishmael
      - Queequeg
  alice_in_wonderland:
    characters:
      - Alice
      - White Rabbit
      - Cheshire Cat
      - Queen of Hearts
  sherlock_holmes:
    characters:
      - Sherlock Holmes
      - Dr. Watson
      - Professor Moriarty
```

### planets.yaml
```yaml
planets:
  - name: Mercury
    moons: 0
    type: terrestrial
  - name: Venus
    moons: 0
    type: terrestrial
  - name: Earth
    moons: 1
    type: terrestrial
  - name: Mars
    moons: 2
    type: terrestrial
  - name: Jupiter
    moons: 95
    type: gas_giant
  - name: Saturn
    moons: 146
    type: gas_giant
  - name: Uranus
    moons: 28
    type: ice_giant
  - name: Neptune
    moons: 16
    type: ice_giant
```

### elements.yaml
```yaml
elements:
  - symbol: H
    name: Hydrogen
    number: 1
    weight: 1.008
  - symbol: He
    name: Helium
    number: 2
    weight: 4.003
  - symbol: Li
    name: Lithium
    number: 3
    weight: 6.941
  - symbol: Be
    name: Beryllium
    number: 4
    weight: 9.012
  - symbol: B
    name: Boron
    number: 5
    weight: 10.81
```
