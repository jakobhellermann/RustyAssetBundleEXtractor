use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Cursor};
use std::path::Path;

use anyhow::Result;
use rabex::files::SerializedFile;
use rabex::files::bundlefile::{BundleFileReader, ExtractionConfig};
use rabex::objects::{ClassId, PPtr};
use rabex::tpk::TpkTypeTreeBlob;
use rabex::typetree::typetree_cache::sync::TypeTreeCache;
use serde_derive::Deserialize;

fn main() -> Result<()> {
    let tpk = TypeTreeCache::new(TpkTypeTreeBlob::embedded());

    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("Expected path to unity bundle argument"))?;
    let config = ExtractionConfig::default().assume_recent_unity();

    let mut reader = BufReader::new(File::open(path)?);
    let mut bundle = BundleFileReader::from_reader(&mut reader, &config)?;

    let export_dir = Path::new("out");
    std::fs::create_dir_all(&export_dir)?;

    while let Some(mut file) = bundle.next_serialized() {
        let data = file.read()?;
        let reader = &mut Cursor::new(data);

        let serialized = SerializedFile::from_reader(reader)?;

        println!(
            "Contains typetree information: {}",
            serialized.m_EnableTypeTree
        );

        for info in serialized.objects() {
            println!("{:?} at {}", info.m_ClassID, info.m_PathID);

            let object = serialized.get_object::<serde_json::Value>(info.m_PathID, &tpk)?;
            let data = object.read(reader)?;
            let name = data["m_Name"].as_str().unwrap();

            let object_path = export_dir.join(name);

            std::fs::write(object_path, serde_json::to_string_pretty(&data)?)?;

            // Objects can be deserialized into any `serde` type
            if info.m_ClassID == ClassId::AssetBundle {
                let asset_bundle = serialized.read::<AssetBundle>(info, &tpk, reader)?;
                println!("Asset Bundle: {:#?}", asset_bundle);
            }
        }
    }

    Ok(())
}

#[derive(Debug, Deserialize, Default)]
#[allow(non_snake_case)]
pub struct AssetBundle {
    pub m_Name: String,
    pub m_PreloadTable: Vec<PPtr>,
    pub m_AssetBundleName: String,
    pub m_IsStreamedSceneAssetBundle: bool,
    pub m_SceneHashes: HashMap<String, String>,
    // ...
}
