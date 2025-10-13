use std::borrow::Cow;

use crate::objects::ClassId;
#[cfg(feature = "embed-tpk")]
use crate::tpk::TpkTypeTreeBlob;
use crate::typetree::TypeTreeNode;
use crate::typetree::TypeTreeProvider;
use crate::unity_version::UnityVersion;

/// [`TypeTreeProvider`] which caches the results from the underlying provider.
///
/// Consider [`sync::TypeTreeCache`] for a thread-safe variant.
pub struct TypeTreeCache<T> {
    pub inner: T,
    typetree_cache: elsa::FrozenMap<ClassId, Box<Option<TypeTreeNode>>>,
}
impl<T: TypeTreeProvider> TypeTreeCache<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            typetree_cache: Default::default(),
        }
    }
}
#[cfg(feature = "embed-tpk")]
impl TypeTreeCache<TpkTypeTreeBlob> {
    pub fn embedded() -> Self {
        Self {
            inner: TpkTypeTreeBlob::embedded(),
            typetree_cache: Default::default(),
        }
    }
}

impl<T: TypeTreeProvider> TypeTreeProvider for TypeTreeCache<T> {
    /// Return the cached reference. Will always be [`Cow::Borrowed`].
    fn get_typetree_node(
        &self,
        class_id: ClassId,
        target_version: &UnityVersion,
    ) -> Option<Cow<'_, TypeTreeNode>> {
        match self.typetree_cache.get(&class_id) {
            Some(value) => value.as_ref().map(Cow::Borrowed),
            None => {
                let tt = self
                    .inner
                    .get_typetree_node(class_id, target_version)
                    .map(Cow::into_owned);
                self.typetree_cache
                    .insert(class_id, Box::new(tt))
                    .as_ref()
                    .map(Cow::Borrowed)
            }
        }
    }
}

/// Thread-safe variant
pub mod sync {
    use std::borrow::Cow;

    use crate::objects::ClassId;
    #[cfg(feature = "embed-tpk")]
    use crate::tpk::TpkTypeTreeBlob;
    use crate::typetree::TypeTreeNode;
    use crate::typetree::typetree_cache::TypeTreeProvider;
    use crate::unity_version::UnityVersion;

    /// [`TypeTreeProvider`] which caches the results from the underlying provider.
    pub struct TypeTreeCache<T> {
        pub inner: T,
        typetree_cache: elsa::sync::FrozenMap<ClassId, Box<Option<TypeTreeNode>>>,
    }

    impl<T: TypeTreeProvider> TypeTreeCache<T> {
        pub fn new(inner: T) -> Self {
            Self {
                inner,
                typetree_cache: Default::default(),
            }
        }
    }

    #[cfg(feature = "embed-tpk")]
    impl TypeTreeCache<TpkTypeTreeBlob> {
        pub fn embedded() -> Self {
            Self {
                inner: TpkTypeTreeBlob::embedded(),
                typetree_cache: Default::default(),
            }
        }
    }

    impl<T: TypeTreeProvider> TypeTreeProvider for TypeTreeCache<T> {
        /// Return the cached reference. Will always be [`Cow::Borrowed`].
        fn get_typetree_node(
            &self,
            class_id: ClassId,
            target_version: &UnityVersion,
        ) -> Option<Cow<'_, TypeTreeNode>> {
            match self.typetree_cache.get(&class_id) {
                Some(value) => value.as_ref().map(Cow::Borrowed),
                None => {
                    let tt = self
                        .inner
                        .get_typetree_node(class_id, target_version)
                        .map(Cow::into_owned);
                    self.typetree_cache
                        .insert(class_id, Box::new(tt))
                        .as_ref()
                        .map(Cow::Borrowed)
                }
            }
        }
    }
}
