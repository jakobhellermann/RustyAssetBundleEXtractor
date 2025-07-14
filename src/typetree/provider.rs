use std::borrow::Cow;

use crate::{UnityVersion, objects::ClassId, tpk::TpkTypeTreeBlob};

use super::TypeTreeNode;

/// Interface for retreiving a type tree for a given [`ClassId`].
///
/// It is implemented by [`TpkTypeTreeBlob`](crate::tpk::TpkTypeTreeBlob),
/// but you should consider using the [`TypeTreeCache`](crate::typetree::typetree_cache::TypeTreeCache)
/// or it's thread-safe variant [`sync::TypeTreeCache`](crate::typetree::typetree_cache::sync::TypeTreeCache)
/// for performance reasons.
pub trait TypeTreeProvider {
    fn get_typetree_node(
        &self,
        class_id: ClassId,
        target_version: UnityVersion,
    ) -> Option<Cow<'_, TypeTreeNode>>;
}

impl TypeTreeProvider for TpkTypeTreeBlob {
    /// Look up and construct a [`TypeTreeNode`] for the class and unity version.
    /// If you call this method often, consider using [`TpkTypeTreeCache`](crate::typetree::typetree_cache::TypeTreeCache).
    fn get_typetree_node(
        &self,
        class_id: ClassId,
        target_version: UnityVersion,
    ) -> Option<Cow<'_, TypeTreeNode>> {
        TpkTypeTreeBlob::get_typetree_node(self, class_id, target_version).map(Cow::Owned)
    }
}
impl<T: TypeTreeProvider> TypeTreeProvider for &T {
    fn get_typetree_node(
        &self,
        class_id: ClassId,
        target_version: UnityVersion,
    ) -> Option<Cow<'_, TypeTreeNode>> {
        (*self).get_typetree_node(class_id, target_version)
    }
}
