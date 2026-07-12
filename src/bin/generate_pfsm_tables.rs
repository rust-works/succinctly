/// Generate PFSM tables for JSON parsing
///
/// This program generates the transition and phi tables used by the Parallel Finite State Machine
/// (PFSM) implementation. The tables are based on the state machine defined in the haskellworks
/// succinct library, which uses a 4-state FSM for JSON parsing.
///
/// State machine:
/// - InJson (0): Initial state, between JSON values
/// - InString (1): Inside a string literal
/// - InEscape (2): After seeing backslash in string
/// - InValue (3): Inside a non-string value (number, true, false, null)
///
/// Phi output encoding (3 bits, matching src/json/standard.rs):
/// - Bit 0 (0b001): BP close - marks closing brackets/braces
/// - Bit 1 (0b010): BP open - marks opening brackets/braces
/// - Bit 2 (0b100): Interest bit - marks structural characters and value starts
///
/// The tables are stored as 256-entry arrays where each entry is a 32-bit word
/// containing 4 bytes, one for each state (InJson=byte 0, InString=byte 1, etc.)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[allow(clippy::enum_variant_names)] // STYLE-0004: PFSM state variants share the In* prefix by design (they mirror parser states)
enum State {
    InJson = 0,
    InString = 1,
    InEscape = 2,
    InValue = 3,
}

/// Character classification functions matching haskellworks implementation
fn is_alphabetic(c: u8) -> bool {
    c.is_ascii_alphabetic()
}

fn is_digit(c: u8) -> bool {
    c.is_ascii_digit()
}

fn is_period(c: u8) -> bool {
    c == b'.'
}

fn is_minus(c: u8) -> bool {
    c == b'-'
}

fn is_plus(c: u8) -> bool {
    c == b'+'
}

fn is_value_char(c: u8) -> bool {
    is_alphabetic(c) || is_digit(c) || is_period(c) || is_minus(c) || is_plus(c)
}

fn is_open(c: u8) -> bool {
    c == b'[' || c == b'{'
}

fn is_close(c: u8) -> bool {
    c == b']' || c == b'}'
}

fn is_delim(c: u8) -> bool {
    c == b',' || c == b':'
}

fn is_double_quote(c: u8) -> bool {
    c == b'"'
}

fn is_backslash(c: u8) -> bool {
    c == b'\\'
}

/// State machine matching our standard cursor implementation
/// Returns (next_state, phi_bits)
///
/// Our phi encoding (matching src/json/standard.rs):
///   Bit 0 (0b001): BP close
///   Bit 1 (0b010): BP open
///   Bit 2 (0b100): Interest bit
///
/// Constants:
///   NONE  = 0b000
///   CLOSE = 0b001  (BP close only)
///   OPEN  = 0b110  (IB + BP open)
///   LEAF  = 0b111  (IB + BP open + BP close)
fn state_machine(c: u8, state: State) -> (State, u8) {
    const NONE: u8 = 0b000;
    const CLOSE: u8 = 0b001; // Just BP close (no IB)
    const OPEN: u8 = 0b110; // IB + BP open
    const LEAF: u8 = 0b111; // IB + BP open + BP close

    match state {
        State::InJson => {
            if is_open(c) {
                (State::InJson, OPEN) // `{` or `[`: IB=1, BP_open=1
            } else if is_close(c) {
                (State::InJson, CLOSE) // `}` or `]`: IB=0, BP_close=1
            } else if is_delim(c) {
                (State::InJson, NONE) // `,` or `:`: no output
            } else if is_value_char(c) {
                (State::InValue, LEAF) // Value start: IB=1, leaf node
            } else if is_double_quote(c) {
                (State::InString, LEAF) // String start: IB=1, leaf node
            } else {
                (State::InJson, NONE) // Whitespace: no output
            }
        }
        State::InString => {
            if is_double_quote(c) {
                (State::InJson, NONE) // String end: no output
            } else if is_backslash(c) {
                (State::InEscape, NONE) // Escape sequence: no output
            } else {
                (State::InString, NONE) // String content: no output
            }
        }
        State::InEscape => {
            (State::InString, NONE) // Escaped char: no output
        }
        State::InValue => {
            if is_open(c) {
                (State::InJson, OPEN) // Value end, `{` or `[` follows
            } else if is_close(c) {
                (State::InJson, CLOSE) // Value end, `}` or `]` follows
            } else if is_delim(c) {
                (State::InJson, NONE) // Value end, delimiter follows
            } else if is_value_char(c) {
                (State::InValue, NONE) // Continue value: no output
            } else {
                (State::InJson, NONE) // Value end, whitespace follows
            }
        }
    }
}

fn generate_transition_table() -> [u32; 256] {
    let mut table = [0u32; 256];

    for byte in 0..=255u8 {
        let mut entry = 0u32;

        // Pack 4 state bytes into one u32
        // Byte 0 = InJson, Byte 1 = InString, Byte 2 = InEscape, Byte 3 = InValue
        for state_val in 0..4 {
            let state = match state_val {
                0 => State::InJson,
                1 => State::InString,
                2 => State::InEscape,
                3 => State::InValue,
                _ => unreachable!(),
            };

            let (next_state, _) = state_machine(byte, state);
            let next_state_byte = next_state as u8;

            // Pack into the appropriate byte position
            entry |= (next_state_byte as u32) << (state_val * 8);
        }

        table[byte as usize] = entry;
    }

    table
}

fn generate_phi_table() -> [u32; 256] {
    let mut table = [0u32; 256];

    for byte in 0..=255u8 {
        let mut entry = 0u32;

        // Pack 4 phi bytes into one u32
        // Byte 0 = InJson, Byte 1 = InString, Byte 2 = InEscape, Byte 3 = InValue
        for state_val in 0..4 {
            let state = match state_val {
                0 => State::InJson,
                1 => State::InString,
                2 => State::InEscape,
                3 => State::InValue,
                _ => unreachable!(),
            };

            let (_, phi) = state_machine(byte, state);

            // Pack into the appropriate byte position
            entry |= (phi as u32) << (state_val * 8);
        }

        table[byte as usize] = entry;
    }

    table
}

fn format_table(table: &[u32; 256], name: &str) -> String {
    let mut output = String::new();
    output.push_str(&format!("pub const {}: [u32; 256] = [\n", name));

    for chunk in table.chunks(8) {
        output.push_str("    ");
        for (i, &entry) in chunk.iter().enumerate() {
            output.push_str(&format!("0x{:08X}", entry));
            if i < chunk.len() - 1 {
                output.push_str(", ");
            }
        }
        output.push_str(",\n");
    }

    output.push_str("];\n");
    output
}

fn main() {
    println!("// Auto-generated PFSM tables for JSON parsing");
    println!("// Generated by: src/bin/generate_pfsm_tables.rs");
    println!("//");
    println!("// Based on the state machine from haskellworks succinct library:");
    println!("// https://github.com/haskell-works/succinct");
    println!("//");
    println!("// Table layout:");
    println!("//   Each entry is a 32-bit word containing 4 bytes (one per state)");
    println!("//   Byte offset 0: InJson state");
    println!("//   Byte offset 1: InString state");
    println!("//   Byte offset 2: InEscape state");
    println!("//   Byte offset 3: InValue state");
    println!("//");
    println!("// Phi bits encoding (3 bits, matching src/json/standard.rs):");
    println!("//   Bit 0 (0b001): BP close - marks closing brackets/braces");
    println!("//   Bit 1 (0b010): BP open - marks opening brackets/braces");
    println!("//   Bit 2 (0b100): IB (Interest Bit) - marks structural chars and value starts");
    println!();

    let transition_table = generate_transition_table();
    println!("{}", format_table(&transition_table, "TRANSITION_TABLE"));
    println!();

    let phi_table = generate_phi_table();
    println!("{}", format_table(&phi_table, "PHI_TABLE"));

    // Print some debug information
    eprintln!("\n=== Table Generation Complete ===");
    eprintln!("\nSample transitions for key characters:");
    for &(ch, desc) in &[
        (b'{', "open brace"),
        (b'}', "close brace"),
        (b'[', "open bracket"),
        (b']', "close bracket"),
        (b'"', "double quote"),
        (b',', "comma"),
        (b':', "colon"),
        (b'a', "letter 'a'"),
        (b'1', "digit '1'"),
        (b' ', "space"),
    ] {
        eprintln!("\n'{}' ({}):", ch as char, desc);
        for state in [
            State::InJson,
            State::InString,
            State::InEscape,
            State::InValue,
        ] {
            let (next, phi) = state_machine(ch, state);
            let bp_close = phi & 1;
            let bp_open = (phi >> 1) & 1;
            let ib = (phi >> 2) & 1;
            eprintln!(
                "  {:?} -> {:?}, phi=0b{:03b} (IB={}, BP_open={}, BP_close={})",
                state, next, phi, ib, bp_open, bp_close
            );
        }
    }
}
