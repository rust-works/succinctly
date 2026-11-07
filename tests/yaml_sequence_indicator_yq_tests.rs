//! #6414: two refusals yq has for the block sequence indicator `-`, which only the yq build makes.
//!
//! A property in front of a `-` on its line (`&t - x`, `a: &t - x`, `- !!seq - x`, `? &t - x`) is no
//! node's property: yq stops with `block sequence entries are not allowed in this context`, wherever
//! the property stands. A tab among the blanks after the `-` (`-⇥x`, `- ⇥x`, `-⇥# c`) is no
//! separation to go-yaml, which skips no tab where a simple key is allowed. The library loader reads
//! both as YAML 1.2 does and keeps reading them, as #6379's tab rule does for `?` and `:`.
//!
//! Inputs are byte literals so no editor or formatter can quietly launder a tab into a space. Every
//! verdict was captured from yq v4.53.3.
//!
//! Run with: `cargo test --test yaml_sequence_indicator_yq_tests`

use succinctly::yaml::YamlIndex;

/// A property on the line of a `-` is refused by yq and read by the library loader.
#[test]
fn a_property_in_front_of_a_dash_on_its_line_is_refused_by_the_yq_build_only() {
    for doc in [
        &b"&t - x\n"[..],
        b"&t - x\n  - y\n",
        b"&t -\n",
        b"&t - - x\n",
        b"&a &b - x\n",
        b"&t !!seq - x\n",
        b"!!seq &t - x\n",
        b"!!seq - x\n",
        b"- &t - x\n",
        b"- !!seq - x\n",
        b"a: &t - x\n",
        b"a:\n  b: &t - x\n",
        b"? &t - x\n: v\n",
        b"&t - a: b\n",
        b"- &t - a: b\n",
        b"a: &t - a: b\n",
    ] {
        assert!(
            YamlIndex::build(doc).is_ok(),
            "{doc:?}: the library reads it"
        );
        assert!(YamlIndex::build_yq(doc).is_err(), "{doc:?}: yq refuses it");
    }
}

/// A property on a line of its own above the `-`, or none, is fine in both.
#[test]
fn a_property_on_its_own_line_above_a_dash_is_read_by_both_builds() {
    for doc in [
        &b"&t\n- x\n"[..],
        b"!!seq\n- x\n",
        b"- &a\n  - x\n",
        b"a: &t\n  - x\n",
        b"a: !!seq\n- x\n",
        b"- &a x\n",
        b"- !!str x\n",
        b"- # c\n- x\n",
        b"-  # c\n",
        b"- - x\n",
    ] {
        assert!(YamlIndex::build(doc).is_ok(), "{doc:?}");
        assert!(YamlIndex::build_yq(doc).is_ok(), "{doc:?}");
    }
}

/// A tab after the dash is refused by yq, comment or not, and read by the library loader.
#[test]
fn a_tab_after_a_dash_is_refused_by_the_yq_build_only() {
    for doc in [
        &b"-\tx\n"[..],
        b"- \tx\n",
        b"-\t\tx\n",
        b"-\t\n",
        b"- \t\n",
        b"-\t# c\n",
        b"- \t# c\n",
        b"-\t- x\n",
        b"-\ta: b\n",
        b"a:\n  -\tx\n",
        b"a:\n  - b\n  -\t# c\n",
        b"- a:\n    -\tx\n",
        b"? -\tx\n: v\n",
    ] {
        assert!(
            YamlIndex::build(doc).is_ok(),
            "{doc:?}: the library reads it"
        );
        assert!(YamlIndex::build_yq(doc).is_err(), "{doc:?}: yq refuses it");
    }
}

/// A tab after what the item holds, or inside it, is not after the dash.
#[test]
fn a_tab_elsewhere_in_an_item_is_read_by_both_builds() {
    for doc in [
        &b"- x\t# c\n"[..],
        b"- a\ty\n",
        b"- \"a\tb\"\n",
        b"- k:\ttab\n",
        b"a:\t# c\n  - x\n",
    ] {
        assert!(YamlIndex::build(doc).is_ok(), "{doc:?}");
        // Whether yq reads each is the CLI test's: only the dash is under test here.
        let _ = YamlIndex::build_yq(doc);
    }
}
