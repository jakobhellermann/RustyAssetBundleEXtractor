//! Utilities for working with the objects of a [`SerializedFile`](crate::files::serializedfile::SerializedFile)
pub mod classes;

mod class_id;
pub mod pptr;

pub use class_id::ClassId;
pub use pptr::{PPtr, TypedPPtr};

/// A trait associating its type to a [`ClassId`].
///
/// ```
/// use rabex::objects::{ClassId, ClassIdType};
/// use rabex::objects::pptr::TypedPPtr;
/// use serde_derive::{Serialize, Deserialize};
/// # #[derive(Debug,Serialize,Deserialize)] struct GameObject;
///
/// #[derive(Debug, Serialize, Deserialize, Clone)]
/// pub struct Transform {
///     pub m_GameObject: TypedPPtr<GameObject>,
///     pub m_LocalRotation: (f32, f32, f32, f32),
///     pub m_LocalPosition: (f32, f32, f32),
///     pub m_LocalScale: (f32, f32, f32),
///     pub m_Children: Vec<TypedPPtr<Transform>>,
///     pub m_Father: TypedPPtr<Transform>,
/// }
/// impl ClassIdType for Transform {
///     const CLASS_ID: ClassId = ClassId::Transform;
/// }
/// ```
pub trait ClassIdType {
    const CLASS_ID: ClassId;
}
