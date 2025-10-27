use std::marker::PhantomData;

use crate::files::SerializedFile;
use crate::files::serializedfile::{FileIdentifier, ObjectRef, Result};
use crate::typetree::TypeTreeProvider;
use serde_derive::{Deserialize, Serialize};

pub type PathId = i64;
pub type FileId = i32;

/// Pointer to another object in this or an external [`SerializedFile`]
#[derive(Debug, Serialize, Deserialize, Default, Copy, Clone, PartialEq, Eq)]
pub struct PPtr {
    /// The [`SerializedFile`] the referenced object belongs to.
    ///
    /// - when `0`, which signals a local PPtr belonging to the same file
    /// - when nonzero, refers to [`SerializedFile::m_Externals`] at `m_FileId-1`
    pub m_FileID: FileId,
    /// Can be zero, for a null [`PPtr`]
    pub m_PathID: PathId,
}

impl PPtr {
    pub fn new(file_id: FileId, path_id: PathId) -> PPtr {
        PPtr {
            m_FileID: file_id,
            m_PathID: path_id,
        }
    }

    /// [`PPtr`] local to its file.
    pub fn local(path_id: PathId) -> PPtr {
        PPtr::new(0, path_id)
    }

    pub fn null() -> PPtr {
        PPtr::new(0, 0)
    }

    pub fn is_null(self) -> bool {
        self == PPtr::null()
    }

    /// Returns `Some` only if `m_PathId` is not 0
    pub fn optional(self) -> Option<PPtr> {
        (self.m_PathID != 0).then_some(self)
    }

    pub fn typed<T>(self) -> TypedPPtr<T> {
        TypedPPtr {
            m_FileID: self.m_FileID,
            m_PathID: self.m_PathID,
            marker: PhantomData,
        }
    }

    /// Force the [`m_FileId`](PPtr::m_FileID) to be zero.
    pub fn make_local(self) -> PPtr {
        PPtr {
            m_FileID: 0,
            m_PathID: self.m_PathID,
        }
    }

    pub fn is_local(self) -> bool {
        self.m_FileID == 0
    }

    pub fn as_local(self) -> Option<Self> {
        self.is_local().then_some(self)
    }

    /// Get a handled to the object referenced by this `PPtr`.
    ///
    /// Only works for local `PPtr`s.
    #[track_caller]
    pub fn deref_local<'a, T>(
        self,
        file: &'a SerializedFile,
        tpk: &'a impl TypeTreeProvider,
    ) -> Result<ObjectRef<'a, T>> {
        assert!(self.is_local(), "Non-local pptr in deref_read_local");
        file.get_object(self.m_PathID, tpk)
    }

    pub fn file_identifier(self, file: &SerializedFile) -> Option<&FileIdentifier> {
        file.m_Externals
            .get(usize::try_from(self.m_FileID - 1).ok()?)
    }
}

/// Typed pointer to another object in this or an external [`SerializedFile`].
#[derive(Serialize, Deserialize)]
pub struct TypedPPtr<T> {
    /// The [`SerializedFile`] the referenced object belongs to.
    ///
    /// - when `0`, which signals a local PPtr belonging to the same file
    /// - when nonzero, refers to [`SerializedFile::m_Externals`] at `m_FileId-1`
    pub m_FileID: FileId,
    /// Can be zero, for a null [`PPtr`]
    pub m_PathID: PathId,
    #[serde(skip)]
    marker: PhantomData<T>,
}

impl<T> Eq for TypedPPtr<T> {}
impl<T> PartialEq for TypedPPtr<T> {
    fn eq(&self, other: &Self) -> bool {
        self.m_FileID == other.m_FileID && self.m_PathID == other.m_PathID
    }
}

impl<T> Default for TypedPPtr<T> {
    fn default() -> Self {
        Self {
            m_FileID: 0,
            m_PathID: 0,
            marker: PhantomData,
        }
    }
}

impl<T: std::fmt::Debug + 'static> std::fmt::Debug for TypedPPtr<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(&format!("TypedPPtr<{}>", std::any::type_name::<T>()))
            .field("m_FileID", &self.m_FileID)
            .field("m_PathID", &self.m_PathID)
            .finish()
    }
}

impl<T> Copy for TypedPPtr<T> {}

impl<T> Clone for TypedPPtr<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> TypedPPtr<T> {
    pub fn new(file_id: FileId, path_id: PathId) -> TypedPPtr<T> {
        TypedPPtr {
            m_FileID: file_id,
            m_PathID: path_id,
            marker: PhantomData,
        }
    }

    /// [`PPtr`] local to its file.
    pub fn local(path_id: PathId) -> TypedPPtr<T> {
        TypedPPtr::new(0, path_id)
    }

    pub fn null() -> TypedPPtr<T> {
        TypedPPtr::default()
    }

    pub fn is_null(self) -> bool {
        self == TypedPPtr::null()
    }

    /// Returns `Some` only if `m_PathId` is not 0
    pub fn optional(self) -> Option<TypedPPtr<T>> {
        (self.m_PathID != 0).then_some(self)
    }

    pub fn untyped(self) -> PPtr {
        PPtr {
            m_FileID: self.m_FileID,
            m_PathID: self.m_PathID,
        }
    }

    pub fn make_local(self) -> TypedPPtr<T> {
        TypedPPtr {
            m_FileID: 0,
            m_PathID: self.m_PathID,
            marker: self.marker,
        }
    }

    pub fn is_local(self) -> bool {
        self.m_FileID == 0
    }

    /// Get a handled to the object referenced by this `PPtr`.
    ///
    /// Only works for local `PPtr`s.
    #[track_caller]
    pub fn deref_local<'a>(
        self,
        file: &'a SerializedFile,
        tpk: &'a impl TypeTreeProvider,
    ) -> Result<ObjectRef<'a, T>> {
        self.untyped().deref_local::<T>(file, tpk)
    }
}
