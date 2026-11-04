//! Raw data (`TypelessData`) reads into `serde_json::Value`, which has no bytes: as an
//! array of numbers.
use std::io::Cursor;

use byteorder::{LittleEndian, WriteBytesExt};
use rabex::serde_typetree;
use rabex::typetree::TypeTreeNode;

#[test]
fn typeless_data_reads_into_json() {
    let node = |ty: &str, name: &str, children| TypeTreeNode {
        m_Type: ty.to_owned(),
        m_Name: name.to_owned(),
        m_MetaFlag: Some(0),
        children,
        ..Default::default()
    };
    let tree = node(
        "Thing",
        "Base",
        vec![node("TypelessData", "image data", vec![])],
    );
    let mut bytes = Vec::new();
    bytes.write_i32::<LittleEndian>(3).unwrap();
    bytes.extend([1, 2, 3]);
    let value: serde_json::Value =
        serde_typetree::from_reader::<_, LittleEndian>(&mut Cursor::new(&bytes), &tree).unwrap();
    assert_eq!(value, serde_json::json!({ "image data": [1, 2, 3] }));
}
