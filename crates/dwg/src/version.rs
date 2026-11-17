use crate::error::{Error, Result};

/// DWG file format versions, ordered oldest to newest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Version {
    R13,
    R14,
    R2000,
    R2004,
    R2007,
    R2010,
    R2013,
    R2018,
}

impl Version {
    pub fn from_magic(magic: &[u8]) -> Result<Version> {
        if magic.len() < 6 || &magic[0..2] != b"AC" {
            return Err(Error::NotDwg);
        }
        Ok(match &magic[0..6] {
            b"AC1012" => Version::R13,
            b"AC1014" => Version::R14,
            b"AC1015" => Version::R2000,
            b"AC1018" => Version::R2004,
            b"AC1021" => Version::R2007,
            b"AC1024" => Version::R2010,
            b"AC1027" => Version::R2013,
            b"AC1032" => Version::R2018,
            other => {
                let s = String::from_utf8_lossy(other).into_owned();
                return Err(match s.as_str() {
                    "AC1.40" | "AC1.50" | "AC2.10" | "AC1001" | "AC1002" | "AC1003" | "AC1004"
                    | "AC1006" | "AC1009" => {
                        Error::Unsupported(format!("pre-R13 DWG ({s}) is not supported"))
                    }
                    _ => Error::Unsupported(format!("unknown DWG version {s:?}")),
                });
            }
        })
    }

    /// The `$ACADVER` string of this version.
    pub fn acadver(self) -> &'static str {
        match self {
            Version::R13 => "AC1012",
            Version::R14 => "AC1014",
            Version::R2000 => "AC1015",
            Version::R2004 => "AC1018",
            Version::R2007 => "AC1021",
            Version::R2010 => "AC1024",
            Version::R2013 => "AC1027",
            Version::R2018 => "AC1032",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Version::R13 => "R13",
            Version::R14 => "R14",
            Version::R2000 => "R2000",
            Version::R2004 => "R2004",
            Version::R2007 => "R2007",
            Version::R2010 => "R2010",
            Version::R2013 => "R2013",
            Version::R2018 => "R2018",
        }
    }
}
