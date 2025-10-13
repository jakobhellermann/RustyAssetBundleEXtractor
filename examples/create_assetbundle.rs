use anyhow::Result;
use rabex::files::bundlefile::BundleFileBuilder;
use rabex::files::bundlefile::CompressionType;
use rabex::files::serializedfile::FileIdentifier;
use rabex::files::serializedfile::build_common_offset_map;
use rabex::files::serializedfile::builder::SerializedFileBuilder;
use rabex::objects::ClassId;
use rabex::objects::ClassIdType;
use rabex::objects::PPtr;
use rabex::tpk::TpkTypeTreeBlob;
use rabex::typetree::typetree_cache::TypeTreeCache;
use serde_derive::Serialize;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Cursor};

fn main() -> Result<()> {
    let tpk_raw = TpkTypeTreeBlob::embedded();
    let tpk = TypeTreeCache::new(&tpk_raw);

    let bundle_name = "bundle";
    let unity_version = "2020.2.2f1".parse().unwrap();
    let mut out = BufWriter::new(File::create("out.unity3d")?);

    let objects = [
        (6, [199, 1210].as_slice()),
        (37, &[660]),
        (38, &[257]),
        (40, &[258]),
    ];

    let common_offset_map = build_common_offset_map(&tpk_raw, &unity_version);

    let mut builder = SerializedFileBuilder::new(&unity_version, &tpk, &common_offset_map, false);

    let mut container = BTreeMap::default();
    for (scene_index, objects) in objects {
        let new_file_idx =
            builder.add_external_uncached(FileIdentifier::new(format!("level{scene_index}")));

        for &obj in objects {
            container.insert(
                format!("test/{scene_index}-{obj}.prefab"),
                AssetInfo {
                    preloadIndex: 0,
                    preloadSize: 0,
                    asset: PPtr::new(new_file_idx, obj),
                },
            );
        }
    }

    let ab = AssetBundle {
        m_Name: bundle_name.to_owned(),
        m_Container: container,
        m_MainAsset: AssetInfo::default(),
        m_RuntimeCompatibility: 1,
        m_IsStreamedSceneAssetBundle: false,
        m_PathFlags: 7,
        ..Default::default()
    };
    builder.add_object_at(1, &ab)?;

    let mut builder_out = Vec::new();
    builder.write(Cursor::new(&mut builder_out))?;

    let mut bundle = BundleFileBuilder::unityfs(7, &unity_version);
    bundle.add_file("CAB-bundle", Cursor::new(builder_out))?;

    bundle.write(&mut out, CompressionType::None)?;

    Ok(())
}

#[derive(Debug, Serialize, Default)]
#[allow(non_snake_case)]
pub struct PreloadData {
    pub m_Name: String,
    pub m_Assets: Vec<PPtr>,
    pub m_Dependencies: Vec<String>,
    pub m_ExplicitDataLayout: bool,
}
impl ClassIdType for PreloadData {
    const CLASS_ID: ClassId = ClassId::PreloadData;
}

#[derive(Debug, Serialize, Default)]
#[allow(non_snake_case)]
pub struct AssetBundle {
    pub m_Name: String,
    pub m_PreloadTable: Vec<PPtr>,
    pub m_Container: BTreeMap<String, AssetInfo>,
    pub m_MainAsset: AssetInfo,
    pub m_RuntimeCompatibility: u32,
    pub m_AssetBundleName: String,
    pub m_Dependencies: Vec<String>,
    pub m_IsStreamedSceneAssetBundle: bool,
    pub m_ExplicitDataLayout: i32,
    pub m_PathFlags: i32,
    pub m_SceneHashes: BTreeMap<String, String>,
}
impl ClassIdType for AssetBundle {
    const CLASS_ID: ClassId = ClassId::AssetBundle;
}

#[derive(Debug, Serialize, Default)]
#[allow(non_snake_case)]
pub struct AssetInfo {
    pub preloadIndex: i32,
    pub preloadSize: i32,
    pub asset: PPtr,
}
