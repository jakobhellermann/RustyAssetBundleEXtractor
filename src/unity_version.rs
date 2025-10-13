use std::str::FromStr;

use num_enum::TryFromPrimitive;

/// A parsed unity version.
///
/// Example: `2023.2.18f1`
#[derive(Clone, PartialEq, PartialOrd)]
pub struct UnityVersion {
    pub major: u16,
    pub minor: u16,
    pub build: u16,
    pub typ: UnityVersionType,
    pub build_number: u8,
    pub trailing_data: String,
}

impl UnityVersion {
    /// major.minor.buildf1
    pub fn new(major: u16, minor: u16, build: u16) -> Self {
        UnityVersion {
            major,
            minor,
            build,
            typ: UnityVersionType::Final,
            build_number: 1,
            trailing_data: String::new(),
        }
    }

    pub fn version_tuple(&self) -> (u16, u16, u16) {
        (self.major, self.minor, self.build)
    }
}

impl std::fmt::Debug for UnityVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::fmt::Display for UnityVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}.{}.{}{}{}",
            self.major,
            self.minor,
            self.build,
            self.typ.char(),
            self.build_number,
        )?;
        if !self.trailing_data.is_empty() {
            write!(f, "-{}", self.trailing_data)?;
        }
        Ok(())
    }
}

#[non_exhaustive]
#[derive(Debug)]
pub struct UnityVersionParseError(String);

impl std::fmt::Display for UnityVersionParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Could not parse unity version {}", self.0)
    }
}

impl std::error::Error for UnityVersionParseError {}

impl FromStr for UnityVersion {
    type Err = UnityVersionParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // TODO: reproduce 2020.2.2f1\n2
        (|| {
            let mut split = s.split('.');
            let major = split.next()?.parse().ok()?;
            let minor = split.next()?.parse().ok()?;
            let rest = split.next()?;

            let i = rest.find(char::is_alphabetic)?;
            let build = &rest[..i];
            let typ = match rest.as_bytes()[i] as char {
                'a' => UnityVersionType::Alpha,
                'b' => UnityVersionType::Beta,
                // TODO china
                'p' => UnityVersionType::Patch,
                'f' => UnityVersionType::Final,
                // TODO experimental
                _ => return None,
            };
            let rest = &rest[i + 1..];

            let (build_number, trailing_data) = rest.split_once('-').unwrap_or((rest, ""));

            Some(UnityVersion {
                major,
                minor,
                build: build.parse().ok()?,
                typ,
                build_number: build_number.parse().ok()?,
                trailing_data: trailing_data.to_owned(),
            })
        })()
        .ok_or_else(|| UnityVersionParseError(s.to_owned()))
    }
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, TryFromPrimitive, PartialEq, PartialOrd)]
pub enum UnityVersionType {
    Alpha = 0,
    Beta = 1,
    China = 2,
    Final = 3,
    Patch = 4,
    Experimental = 5,
}

impl UnityVersionType {
    pub fn char(self) -> char {
        match self {
            UnityVersionType::Alpha => 'a',
            UnityVersionType::Beta => 'b',
            UnityVersionType::China => 'c', // ?
            UnityVersionType::Final => 'f',
            UnityVersionType::Patch => 'p',        // ?
            UnityVersionType::Experimental => 'e', // ?
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{UnityVersion, UnityVersionType};

    #[test]
    fn unity_version_simple() {
        let version: UnityVersion = "2022.2.2a13".parse().unwrap();

        assert_eq!(
            version,
            UnityVersion {
                major: 2022,
                minor: 2,
                build: 2,
                typ: UnityVersionType::Alpha,
                build_number: 13,
                trailing_data: "".into(),
            }
        )
    }

    #[test]
    fn unity_version_complex() {
        let version: UnityVersion = "6000.0.50f1-uum-100966-branch1".parse().unwrap();

        assert_eq!(
            version,
            UnityVersion {
                major: 6000,
                minor: 0,
                build: 50,
                typ: UnityVersionType::Final,
                build_number: 1,
                trailing_data: "uum-100966-branch1".into()
            }
        )
    }
}
