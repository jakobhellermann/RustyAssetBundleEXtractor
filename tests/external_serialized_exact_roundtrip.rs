mod external_test_data;

use std::io::Cursor;
use std::path::Path;

use anyhow::{Context, Result, bail};
use rabex::files::{SerializedFile, serializedfile};
use rabex::tpk::TpkTypeTreeBlob;

#[test]
fn external_roundtrip_levels() -> Result<()> {
    let tt = TpkTypeTreeBlob::embedded();

    for game_path in external_test_data::EXTERNAL_GAME_PATHS {
        for entry in std::fs::read_dir(game_path)? {
            let entry = entry?;
            let filename = entry.file_name();
            let filename = filename.to_str().unwrap();
            if !filename.starts_with("level") {
                continue;
            }
            assert_roundtrip(&entry.path(), &tt)
                .with_context(|| format!("Could not roundtrip {}", entry.file_name().display()))?;
        }
    }
    Ok(())
}

fn assert_roundtrip(path: &Path, tt: &TpkTypeTreeBlob) -> Result<()> {
    println!("{}", path.display());
    let data = std::fs::read(path)?;

    let reader = &mut Cursor::new(data.as_slice());
    let serialized = SerializedFile::from_reader(reader)?;

    let mut out = Cursor::new(Vec::new());

    let common_offset_map =
        serializedfile::build_common_offset_map(&tt, serialized.m_UnityVersion.as_ref().unwrap());
    serializedfile::write_serialized(&mut out, &serialized, reader.get_ref(), &common_offset_map)?;

    let start = 0;
    let up_to = out.get_ref().len().min(data.len());

    let left = &data[start..up_to];
    let right = &out.get_ref()[start..up_to];

    if left != right {
        // let comparison = pretty_assertions::Comparison::new(left, right);
        // bail!("Failed to roundtrip: {}", comparison);
        bail!("error roundtripping {}", path.display());
    }

    // unity aligns at begin, unitypy at end of loop
    assert_eq!(data.len(), out.get_ref().len());

    Ok(())
}
