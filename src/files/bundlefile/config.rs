use crate::unity_version::UnityVersion;

#[derive(Default)]
pub struct ExtractionConfig {
    pub unitycn_key: Option<[u8; 16]>,
    pub fallback_unity_version: Option<UnityVersion>,
}

impl ExtractionConfig {
    pub fn new(
        unitycn_key: Option<[u8; 16]>,
        fallback_unity_version: Option<UnityVersion>,
    ) -> Self {
        Self {
            unitycn_key,
            fallback_unity_version,
        }
    }

    pub fn with_unitycn_key(mut self, key: [u8; 16]) -> Self {
        self.unitycn_key = Some(key);
        self
    }
}
