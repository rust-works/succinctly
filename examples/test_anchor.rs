use succinctly::yaml::YamlIndex;
use succinctly::yaml::YamlValue;

fn main() {
    // Test 6KGN: anchor for empty node
    let yaml = b"---\na: &anchor\nb: *anchor";
    println!("Input: {:?}", String::from_utf8_lossy(yaml));

    let index = YamlIndex::build(yaml).unwrap();

    println!("\n=== Anchors ===");
    if let Some(anchor_pos) = index.get_anchor_bp_pos("anchor") {
        println!("Anchor 'anchor' points to bp_pos={}", anchor_pos);
        let cursor = index.cursor_at(anchor_pos, yaml);
        println!("  text_pos: {:?}", cursor.text_position());
    }

    println!("\n=== Value extraction ===");
    let root = index.root(yaml);
    if let Some(doc) = root.first_child() {
        if let YamlValue::Mapping(fields) = doc.value() {
            for field in fields {
                let key = field.key();
                let val = field.value();
                println!("Key: {:?}", key);
                println!("Value: {:?}", val);
            }
        }
    }
}
