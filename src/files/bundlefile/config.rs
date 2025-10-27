use crate::unity_version::UnityVersion;

/// Versions below 2020.x.x, 2020.3.34, 2020.3.2, 2022.1.1 use a different format
/// for archive flags. If the bundle doesn't specify its unity version,
/// then it's unclear whether the new or old version should be used.
/// If you know the unity version (e.g.) if this is a addressables bundle,
/// you can specify it here.
#[derive(Default)]
pub enum FallbackUnityVersion {
    /// Assume the file is newer than `2020.x.x, 2020.3.34, 2020.3.2, 2022.1.1`
    Current,
    /// Specify the exact unity version in use.
    Exact(UnityVersion),
    /// Error if the file doesn't specify its unity version.
    #[default]
    Error,
}

#[derive(Default)]
pub struct ExtractionConfig {
    pub unitycn_key: Option<[u8; 16]>,
    pub fallback_unity_version: FallbackUnityVersion,
}

impl ExtractionConfig {
    pub fn new(
        unitycn_key: Option<[u8; 16]>,
        fallback_unity_version: FallbackUnityVersion,
    ) -> Self {
        Self {
            unitycn_key,
            fallback_unity_version,
        }
    }

    /// Assume the file is not older than `2020.x.x, 2020.3.34, 2020.3.2, 2022.1.1`.
    pub fn assume_recent_unity(mut self) -> Self {
        self.fallback_unity_version = FallbackUnityVersion::Current;
        self
    }

    /// Specify the unity version of the bundle in case it doesn't include it.
    pub fn with_fallback_unity_version(mut self, fallback_unity_version: UnityVersion) -> Self {
        self.fallback_unity_version = FallbackUnityVersion::Exact(fallback_unity_version);
        self
    }

    pub fn with_unitycn_key(mut self, key: [u8; 16]) -> Self {
        self.unitycn_key = Some(key);
        self
    }
}
