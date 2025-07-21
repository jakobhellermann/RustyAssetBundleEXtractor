use crate::tpk2rust::{Derives, Tpk2Rust};
use anyhow::Result;
use rabex::{UnityVersion, tpk::TpkTypeTreeBlob};
use std::str::FromStr;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let version = args
        .next()
        .as_deref()
        .map(UnityVersion::from_str)
        .transpose()?;

    let tpk = TpkTypeTreeBlob::embedded();
    let mut tpk2rust = Tpk2Rust::new(&tpk, version.as_ref(), Derives::empty());

    let types = ["GameObject", "Transform", "AssetBundle", "MonoBehaviour"];
    for name in types {
        tpk2rust.add_by_name(name);
    }

    print!("{}", tpk2rust.to_rust_code());

    Ok(())
}

mod tpk2rust {
    use std::{borrow::Cow, io::Write};

    use indexmap::{IndexMap, IndexSet};
    use rabex::{UnityVersion, objects::ClassId, tpk::TpkTypeTreeBlob, typetree::TypeTreeNode};
    use rustc_hash::FxHashMap;

    pub struct Tpk2Rust<'tt> {
        version: UnityVersion,

        tpk: &'tt TpkTypeTreeBlob,
        name_lookup: FxHashMap<&'tt str, ClassId>,

        derives: Derives,
        types: IndexMap<&'tt str, (String, Derives)>,
        pptrs: IndexSet<&'tt str>,

        queue: Vec<(&'tt TypeTreeNode, Derives)>,
    }
    impl<'tt> Tpk2Rust<'tt> {
        pub fn new(
            tpk: &'tt TpkTypeTreeBlob,
            version: Option<&UnityVersion>,
            derives: Derives,
        ) -> Self {
            let version = version.unwrap_or(tpk.versions.last().unwrap());

            let name_lookup: FxHashMap<_, _> = tpk
                .class_information
                .iter()
                .filter_map(|(class_id, data)| {
                    let (_, class) = data.last()?;
                    let class = class.as_ref()?;

                    class.release_root_node?;

                    let name = tpk.string_buffer[class.name as usize].as_str();
                    Some((name, *class_id))
                })
                .collect();

            Tpk2Rust {
                version: version.clone(),
                tpk,
                name_lookup,
                derives,
                types: Default::default(),
                pptrs: Default::default(),
                queue: Vec::new(),
            }
        }

        pub fn tt_by_name(&self, name: &str) -> Option<TypeTreeNode> {
            self.tpk
                .get_typetree_node(*self.name_lookup.get(name)?, &self.version)
        }

        pub fn to_rust_code(mut self) -> String {
            let mut out = String::new();

            for pptr in self
                .pptrs
                .extract_if(.., |x| self.name_lookup.contains_key(x))
                .collect::<Vec<_>>()
            {
                self.add_by_name(pptr);
            }

            out.push_str("#![allow(non_snake_case, dead_code)]\n");
            out.push_str("use rabex::objects::TypedPPtr;\n");
            if self.derives.contains(Derives::Deserialize) {
                out.push_str("use serde_derive::Deserialize;\n");
            }
            if self.derives.contains(Derives::Serialize) {
                out.push_str("use serde_derive::Serialize;\n");
            }
            out.push_str("use std::collections::HashMap;\n");
            out.push_str("\n");

            for (ty_rust, derives) in self.types.values() {
                out.push_str(&derives.to_rust_code());
                out.push_str(ty_rust);
            }

            for pptr in &self.pptrs {
                if self.types.contains_key(pptr) {
                    continue;
                }

                out.push_str(&format!("type {pptr} = ();\n"));
            }

            out
        }

        pub fn add_by_name(&mut self, name: &str) {
            let ty = self
                .tt_by_name(name)
                .expect(&format!("Couldn't find type '{name}'"));
            let ty = Box::leak(Box::new(ty));
            self.add(ty);
        }

        pub fn add(&mut self, ty: &'tt TypeTreeNode) {
            self.add_with_derives(ty, self.derives)
        }

        fn add_with_derives(&mut self, ty: &'tt TypeTreeNode, derives: Derives) {
            self.add_shallow(ty, derives);
            self.add_queued();
        }

        fn add_queued(&mut self) {
            for (dep, derives) in std::mem::take(&mut self.queue) {
                match Kind::from(dep) {
                    Kind::Primitive(_) => {}
                    Kind::Pair(..) => todo!(),
                    Kind::Map(..) => todo!(),
                    Kind::List(item_type) => self.add_with_derives(item_type, derives),
                    Kind::Other(other) => self.add_with_derives(other, derives),
                    Kind::PPtr(pptr) => self.add_pptr(pptr),
                }
            }
        }

        fn add_pptr(&mut self, pptr: &'tt str) {
            self.pptrs.insert(pptr);
        }

        fn add_shallow(&mut self, ty: &'tt TypeTreeNode, derives: Derives) {
            if let Some(present) = self.types.get_mut(ty.m_Type.as_str()) {
                present.1 |= derives;
                return;
            }

            let mut out = Vec::new();

            let _ = writeln!(out, "struct {} {{", ty.m_Type);
            for child in &ty.children {
                let rust_ty = self.type_to_rust_usage(child, derives);

                let (serde_rename, rust_name) = match escape_name(&child.m_Name) {
                    Escape::Ok(val) => (None, val.to_owned()),
                    Escape::RustKw(val) => (None, format!("r#{val}")),
                    Escape::Rename(val) => (Some(child.m_Name.as_str()), val),
                };
                if self
                    .derives
                    .intersects(Derives::Serialize | Derives::Deserialize)
                    && let Some(serde_rename) = serde_rename
                {
                    let _ = writeln!(out, "    #[serde(rename = \"{serde_rename}\")]");
                }

                let _ = writeln!(out, "    {rust_name}: {rust_ty},");
            }
            let _ = writeln!(out, "}}");

            self.types
                .insert(&ty.m_Type, (String::from_utf8(out).unwrap(), derives));
        }

        fn add_queue(&mut self, ty: &'tt TypeTreeNode, derives: Derives) {
            self.queue.push((ty, derives));
        }

        fn type_to_rust_usage(&mut self, ty: &'tt TypeTreeNode, derives: Derives) -> Cow<'tt, str> {
            match Kind::from(ty) {
                Kind::Primitive(primitive) => Cow::Borrowed(primitive),
                Kind::Pair(a, b) => {
                    let mut out = String::from("(");
                    out.push_str(&self.type_to_rust_usage(a, derives));
                    out.push_str(", ");
                    out.push_str(&self.type_to_rust_usage(b, derives));
                    out.push(')');
                    Cow::Owned(out)
                }
                Kind::Map(key, value) => Cow::Owned(format!(
                    "HashMap<{}, {}>",
                    self.type_to_rust_usage(key, derives | Derives::Hash),
                    self.type_to_rust_usage(value, derives)
                )),
                Kind::List(type_tree_node) => Cow::Owned(format!(
                    "Vec<{}>",
                    self.type_to_rust_usage(type_tree_node, derives)
                )),
                Kind::Other(other) => {
                    self.add_queue(other, derives);
                    Cow::Borrowed(&other.m_Type)
                }
                Kind::PPtr(pptr_name) => {
                    self.add_queue(ty, derives);
                    Cow::Owned(format!("TypedPPtr<{pptr_name}>"))
                }
            }
        }
    }

    fn escape_name(name: &str) -> Escape<'_> {
        if ["type", "loop"].contains(&name) {
            return Escape::RustKw(name);
        }

        let invalid_chars = |c: char| matches!(c, '[' | ' ');
        if name.contains(invalid_chars) {
            return Escape::Rename(
                name.replace(']', "")
                    .replace(invalid_chars, "_")
                    .replace("__", "_"),
            );
        }

        Escape::Ok(name)
    }

    enum Escape<'a> {
        Ok(&'a str),
        RustKw(&'a str),
        Rename(String),
    }

    enum Kind<'a> {
        Primitive(&'static str),
        Pair(&'a TypeTreeNode, &'a TypeTreeNode),
        Map(&'a TypeTreeNode, &'a TypeTreeNode),
        List(&'a TypeTreeNode),
        Other(&'a TypeTreeNode),
        PPtr(&'a str),
    }

    impl<'a> From<&'a TypeTreeNode> for Kind<'a> {
        fn from(ty: &'a TypeTreeNode) -> Self {
            if let "string" = ty.m_Type.as_str() {
                Kind::Primitive("String")
            }
            // array
            else if let [child] = ty.children.as_slice()
                && child.m_Type == "Array"
            {
                // map
                if let [_, data] = child.children.as_slice()
                    && data.m_Type == "pair"
                    && let [key, value] = data.children.as_slice()
                {
                    Kind::Map(key, value)
                } else {
                    Kind::List(&child.children[1])
                }
            } else if let Some(pptr_name) = ty.m_Type.strip_prefix("PPtr<") {
                let pptr_name = pptr_name.strip_suffix('>').unwrap();
                Kind::PPtr(pptr_name)
            } else {
                match ty.m_Type.as_str() {
                    "bool" => Kind::Primitive("bool"),
                    "UInt8" => Kind::Primitive("u8"),
                    "UInt16" | "unsigned short" => Kind::Primitive("u16"),
                    "UInt32" | "unsigned int" | "Type*" => Kind::Primitive("u32"),
                    "UInt64" | "unsigned long long" | "FileSize" => Kind::Primitive("u64"),
                    "SInt8" => Kind::Primitive("i8"),
                    "SInt16" | "short" => Kind::Primitive("i16"),
                    "SInt32" | "int" => Kind::Primitive("i32"),
                    "SInt64" | "long long" => Kind::Primitive("i64"),
                    "float" => Kind::Primitive("f32"),
                    "double" => Kind::Primitive("f64"),
                    "char" => Kind::Primitive("char"),
                    "pair" => {
                        let [a, b] = ty.children.as_slice() else {
                            panic!("pair with {} elements", ty.children.len());
                        };
                        Kind::Pair(a, b)
                    }
                    _ => Kind::Other(ty),
                }
            }
        }
    }

    bitflags::bitflags! {
        #[derive(Default, Clone, Copy, Debug)]
        pub struct Derives: u32 {
            const Serialize = 0b0001;
            const Deserialize = 0b0010;
            const Hash = 0b0100;
        }
    }

    impl Derives {
        fn to_rust_code(self) -> String {
            let mut out = String::new();
            if !self.is_empty() {
                out.push_str("#[derive(");
                if self.contains(Derives::Serialize) {
                    out.push_str("Serialize, ");
                }
                if self.contains(Derives::Deserialize) {
                    out.push_str("Deserialize, ");
                }
                if self.contains(Derives::Hash) {
                    out.push_str("PartialEq, Eq, std::hash::Hash, ");
                }
                out.truncate(out.len() - 2);
                out.push_str(")]\n");
            }

            out
        }
    }
}
