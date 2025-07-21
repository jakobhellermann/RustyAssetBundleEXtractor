#![allow(non_snake_case)]
use anyhow::Result;
use rabex::{
    files::{
        SerializedFile,
        bundlefile::{BundleFileReader, ExtractionConfig},
    },
    objects::{ClassId, ClassIdType, TypedPPtr},
    tpk::TpkTypeTreeBlob,
    typetree::{TypeTreeProvider, typetree_cache::TypeTreeCache},
};
use rustc_hash::FxHashMap;
use serde_derive::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Cursor, Read, Seek},
};

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("Expected path to unity bundle argument"))?;
    let data = &mut File::open(path)?;

    let tpk = &TypeTreeCache::new(TpkTypeTreeBlob::embedded());

    if let Ok(mut reader) = BundleFileReader::from_reader(&mut *data, &ExtractionConfig::default())
    {
        while let Some(mut file) = reader.next() {
            let mut data = Cursor::new(file.read()?);
            let hierarchy = get_all(&mut data, tpk)?;
            println!("{}", serde_json::to_string_pretty(&hierarchy)?);
        }
    } else {
        let hierarchy = get_all(data, tpk)?;
        println!("{}", serde_json::to_string_pretty(&hierarchy)?);
    }

    Ok(())
}

fn get_all(
    data: &mut (impl Read + Seek),
    tpk: &impl TypeTreeProvider,
) -> Result<Vec<HierarchyObject>> {
    let file = SerializedFile::from_reader(data)?;
    let transforms = file
        .objects_of::<Transform>(tpk)
        .map(|obj| Ok((obj.info.m_PathID, obj.read(data)?)))
        .collect::<Result<FxHashMap<_, _>>>()?;
    let mut all = transforms
        .values()
        .filter(|transform| transform.m_Father.is_null())
        .map(|root| get(root, &file, tpk, data, 0))
        .collect::<Result<Vec<_>>>()?;
    all.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(all)
}

fn get(
    root: &Transform,
    file: &SerializedFile,
    tpk: &impl TypeTreeProvider,
    reader: &mut (impl Read + Seek),
    indent: usize,
) -> Result<HierarchyObject> {
    let go = root.m_GameObject.deref_local(&file, tpk)?.read(reader)?;
    let mut children = root
        .m_Children
        .iter()
        .map(|child| {
            let child = child.deref_local(&file, tpk)?.read(reader)?;
            get(&child, file, tpk, reader, indent + 1)
        })
        .collect::<Result<Vec<_>>>()?;
    children.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(HierarchyObject {
        name: go.m_Name,
        children,
    })
}

#[derive(Debug, Serialize, Clone)]
pub struct HierarchyObject {
    name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<HierarchyObject>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Transform {
    pub m_GameObject: TypedPPtr<GameObject>,
    // pub m_LocalRotation: (f32, f32, f32, f32),
    // pub m_LocalPosition: (f32, f32, f32),
    // pub m_LocalScale: (f32, f32, f32),
    pub m_Children: Vec<TypedPPtr<Transform>>,
    pub m_Father: TypedPPtr<Transform>,
}
impl ClassIdType for Transform {
    const CLASS_ID: ClassId = ClassId::Transform;
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GameObject {
    // pub m_Component: Vec<ComponentPair>,
    // pub m_Layer: u32,
    pub m_Name: String,
    // pub m_Tag: u16,
    // pub m_IsActive: bool,
}
impl ClassIdType for GameObject {
    const CLASS_ID: ClassId = ClassId::GameObject;
}
