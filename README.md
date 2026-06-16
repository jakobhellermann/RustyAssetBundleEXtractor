# RustyAssetBundleEXtractor (rabex)

[![Latest Version]][crates.io] [![Docs]][docs.rs] [![License_MIT]][license_mit] [![License_APACHE]][license_apache] 

[Latest Version]: https://img.shields.io/crates/v/rabex.svg
[crates.io]: https://crates.io/crates/rabex
[Docs]: https://docs.rs/rabex/badge.svg
[docs.rs]: https://docs.rs/crate/rabex/
[License_MIT]: https://img.shields.io/badge/License-MIT-yellow.svg
[license_mit]: https://raw.githubusercontent.com/UniversalGameExtraction/RustyAssetBundleEXtractor/main/LICENSE-MIT
[License_APACHE]: https://img.shields.io/badge/License-Apache%202.0-blue.svg
[license_apache]: https://raw.githubusercontent.com/UniversalGameExtraction/RustyAssetBundleEXtractor/main/LICENSE-APACHE

A crate for working with Unity Engine asset files. It supports reading and writing bundle files and serialized files, as well as reading typetrees with serde integration.

Maintained fork of [UniversalGameExtraction/RustyAssetBundleEXtractor](https://github.com/UniversalGameExtraction/RustyAssetBundleEXtractor), and source of the [rabex](https://crates.io/crates/rabex) crate on crates.io.

## Examples

**Parsing an AssetBundle and dumping its objects**

```rust
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

        for object in serialized.objects() {
            println!("{:?} at {}", object.m_ClassID, object.m_PathID);

            let data = serialized.read::<serde_json::Value>(object, &tpk, reader)?;
            let name = data["m_Name"].as_str().unwrap();

            let object_path = export_dir.join(name);

            std::fs::write(object_path, serde_json::to_string_pretty(&data)?)?;

            // Objects can be deserialized into any `serde` type
            if object.m_ClassID == ClassId::AssetBundle {
                let asset_bundle = serialized.read::<AssetBundle>(object, &tpk, reader)?;
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
```

**Creating an assetbundle from scratch using `BundleFileBuilder` and `SerializedFileBuilder`**

[examples/create_assetbundle.rs](./examples/create_assetbundle.rs)

## Related Projects
- [rabex-env](https://github.com/jakobhellermann/rabex-env): Higher-level wrapper around this crate, with support for resolving `PPtr` dependencies through different files, centered around the `Environment` abstraction.
- [unity-scene-repacker](https://github.com/jakobhellermann/unity-scene-repacker/): Command line tool for repacking unity scenes and asset bundles into distilled versions suitable for loading certain objects in mods
- [steam-multiversion-viewer](https://github.com/jakobhellermann/steam-multiversion-viewer): [wip] Visual exploration tool for steam game versions, with support for reading and structurally diffing unity game files.

## License

RustyAssetBundleEXtractor is licensed under either of [Apache License, Version 2.0](LICENSE-APACHE.d) or [MIT license](LICENSE-MIT.d)
at your option.
