//! Exercises the low-level MonoBehaviour builder API (`add_monobehaviour_type` +
//! `add_monobehaviour_with`). MonoBehaviours can't go through `add_object_at`
//! because their `SerializedType` needs script wiring, so this checks that the
//! wiring round-trips: the written file re-reads, resolves the behaviour back to
//! its `MonoScript`, and serializes byte-for-byte identically.
#![cfg(feature = "embed-tpk")]

use std::io::Cursor;

use rabex::files::SerializedFile;
use rabex::files::serializedfile::builder::SerializedFileBuilder;
use rabex::files::serializedfile::{self};
use rabex::objects::pptr::FileId;
use rabex::objects::{ClassId, ClassIdType, PPtr};
use rabex::typetree::TypeTreeProvider;
use rabex::typetree::typetree_cache::TypeTreeCache;
use serde_derive::Serialize;

#[derive(Serialize)]
#[allow(non_snake_case)]
struct MonoScript {
    m_Name: String,
    m_ExecutionOrder: i32,
    m_PropertiesHash: [u8; 16],
    m_ClassName: String,
    m_Namespace: String,
    m_AssemblyName: String,
}
impl ClassIdType for MonoScript {
    const CLASS_ID: ClassId = ClassId::MonoScript;
}

#[derive(Serialize)]
#[allow(non_snake_case)]
struct MonoBehaviour {
    m_GameObject: PPtr,
    m_Enabled: u8,
    m_Script: PPtr,
    m_Name: String,
}

#[test]
fn monobehaviour_wiring_roundtrips() {
    let version = "2022.3.0f1".parse().unwrap();
    let tpk = TypeTreeCache::embedded();
    let common = serializedfile::build_common_offset_map(&tpk.inner, &version);
    let mut sfb = SerializedFileBuilder::new(&version, &tpk, &common, true);

    let script_id = sfb
        .add_object(&MonoScript {
            m_Name: "PlayMakerFSM".to_owned(),
            m_ExecutionOrder: 0,
            m_PropertiesHash: [0; 16],
            m_ClassName: "PlayMakerFSM".to_owned(),
            m_Namespace: String::new(),
            m_AssemblyName: "Assembly-CSharp.dll".to_owned(),
        })
        .unwrap();
    let script = PPtr::new(FileId::LOCAL, script_id);

    // Embed a MonoBehaviour type tree stamped with the script class name so
    // readers deserialize straight off it.
    let mut mb_tt = tpk
        .get_typetree_node(ClassId::MonoBehaviour, &version)
        .unwrap()
        .into_owned();
    mb_tt.m_Type = "PlayMakerFSM".to_owned();
    let mb_type = sfb.add_monobehaviour_type(script, Some(mb_tt));

    let mb_id = sfb
        .add_monobehaviour_with_type(
            &MonoBehaviour {
                m_GameObject: PPtr::default(),
                m_Enabled: 1,
                m_Script: script,
                m_Name: String::new(),
            },
            mb_type,
        )
        .unwrap();

    let bytes = sfb.write_vec().unwrap();

    // Re-read: the behaviour resolves back to its MonoScript via m_ScriptTypes.
    let file = SerializedFile::from_reader(&mut Cursor::new(&bytes)).unwrap();
    let mb_info = file
        .objects()
        .find(|o| o.m_ClassID == ClassId::MonoBehaviour)
        .expect("MonoBehaviour object");
    assert_eq!(mb_info.m_PathID, mb_id);
    assert_eq!(file.script_type(mb_info), Some(script));

    // The whole file serializes back byte-for-byte.
    let com =
        serializedfile::build_common_offset_map(&tpk.inner, file.m_UnityVersion.as_ref().unwrap());
    let mut out = Cursor::new(Vec::new());
    serializedfile::write_serialized(&mut out, &file, &bytes, &com).unwrap();
    assert_eq!(out.into_inner(), bytes);
}
