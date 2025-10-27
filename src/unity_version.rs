use std::str::FromStr;

use num_enum::TryFromPrimitive;

/// A parsed unity version.
///
/// Example: `2023.2.18f1`
#[derive(Copy, Clone, PartialEq, PartialOrd)]
pub struct UnityVersion {
    pub major: u16,
    pub minor: u16,
    pub build: u16,
    pub typ: UnityVersionType,
    pub build_number: u8,
}

impl UnityVersion {
    pub fn version_tuple(self) -> (u16, u16, u16) {
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
        )
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
            let build_number = &rest[i + 1..];

            let build_number = if let Some((build_number, _)) = build_number.split_once('\n') {
                // ?? 2020.2.2f1\n2
                build_number
            } else {
                build_number
            };

            Some(UnityVersion {
                major,
                minor,
                build: build.parse().ok()?,
                typ,
                build_number: build_number.parse().ok()?,
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
