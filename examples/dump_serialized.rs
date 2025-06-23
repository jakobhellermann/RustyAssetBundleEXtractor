use anyhow::Result;
use rabex::{files::SerializedFile, tpk::TpkTypeTreeBlob, typetree::TypeTreeCache};
use std::fs::File;

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("Expected path to unity bundle argument"))?;
    let data = &mut File::open(path)?;

    let tpk = TypeTreeCache::new(TpkTypeTreeBlob::embedded());

    let file = SerializedFile::from_reader(data)?;
    for obj in file.objects() {
        let value = file.read::<serde_json::Value>(obj, &tpk, data)?;

        println!("{value:#?}");
        let value = file.read::<serde_json::Value>(obj, &tpk, data);
        match value {
            Ok(value) => {
                let serialized = serde_json::to_string_pretty(&value)?;
                println!("{serialized}")
            }
            Err(e) => eprintln!("{:?}", anyhow::Error::from(e)),
        };
    }

    Ok(())
}
