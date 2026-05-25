#![allow(non_snake_case)]
use anyhow::Result;
use rabex::files::SerializedFile;
use rabex::files::bundlefile::{BundleFileReader, ExtractionConfig};
use rabex::objects::{ClassId, ClassIdType, PPtr, TypedPPtr};
use rabex::typetree::TypeTreeProvider;
use rabex::typetree::typetree_cache::TypeTreeCache;
use serde_derive::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};

fn main() -> Result<()> {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        anyhow::bail!("Expected path(s) to unity bundle argument");
    }

    let tpk = &TypeTreeCache::embedded();

    for path in &paths {
        if paths.len() > 1 {
            println!("=== {path} ===");
        }
        let data = &mut File::open(&path)?;

        if let Ok(mut reader) =
            BundleFileReader::from_reader(&mut *data, &ExtractionConfig::default())
        {
            while let Some(mut file) = reader.next() {
                let mut data = Cursor::new(file.read()?);
                print_serialize_hierarchy(&mut data, tpk)?;
            }
        } else {
            data.seek(SeekFrom::Start(0))?;
            print_serialize_hierarchy(data, tpk)?;
        }
    }

    Ok(())
}

fn print_serialize_hierarchy(
    data: &mut (impl Read + Seek),
    tpk: &impl TypeTreeProvider,
) -> Result<(), anyhow::Error> {
    let file = SerializedFile::from_reader(data)?;
    let transforms = file
        .objects_of::<Transform>(tpk)
        .map(|obj| Ok((obj.info.m_PathID, obj.read(data)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    for root in transforms
        .values()
        .filter(|transform| transform.m_Father.is_null())
    {
        print_object_hierarchy(root, &file, tpk, data, 0)?;
    }
    Ok(())
}

fn print_object_hierarchy(
    root: &Transform,
    file: &SerializedFile,
    tpk: &impl TypeTreeProvider,
    reader: &mut (impl Read + Seek),
    indent: usize,
) -> Result<()> {
    let go = root.m_GameObject.deref_local(file, tpk)?.read(reader)?;
    println!("{}{}", "  ".repeat(indent), go.m_Name);

    for component in &go.m_Component {
        let component = component
            .component
            .deref_local::<serde_json::Value>(file, tpk)?;
        println!("{}- {:?}", "  ".repeat(indent), component.info.m_ClassID);
    }

    for child in &root.m_Children {
        let child = child.deref_local(file, tpk)?.read(reader)?;
        print_object_hierarchy(&child, file, tpk, reader, indent + 1)?;
    }
    Ok(())
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
    pub m_Component: Vec<ComponentPair>,
    // pub m_Layer: u32,
    pub m_Name: String,
    // pub m_Tag: u16,
    // pub m_IsActive: bool,
}
impl ClassIdType for GameObject {
    const CLASS_ID: ClassId = ClassId::GameObject;
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ComponentPair {
    pub component: PPtr,
}

#[derive(Debug, Deserialize)]
pub struct Named {
    pub m_Name: Option<String>,
}
