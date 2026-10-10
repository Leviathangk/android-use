use crate::{
    api::{Code, Error, Result},
    device::{atomic, Paths},
};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

pub const MAX_ARTIFACT: usize = 16 * 1024 * 1024;

pub struct Artifacts {
    paths: Paths,
}

impl Artifacts {
    pub fn new(paths: Paths) -> Self {
        Self { paths }
    }
    pub fn put(&self, bytes: &[u8]) -> Result<Box<str>> {
        if bytes.len() > MAX_ARTIFACT {
            return Err(Error::new(Code::Bounds, "artifact exceeds 16 MiB"));
        }
        let id = content_id(bytes);
        let path = self.paths.artifacts.join(&id);
        if !path.exists() {
            atomic(&path, bytes)?;
        }
        Ok(id.into_boxed_str())
    }
    /// Stores media under a content-addressed name with a real extension so
    /// a person or shell agent can open it, and returns the absolute path.
    pub fn export(&self, bytes: &[u8], extension: &str) -> Result<PathBuf> {
        if bytes.is_empty() || bytes.len() > MAX_ARTIFACT || extension.is_empty() || !extension.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(Error::new(Code::Bounds, "exported artifact is empty, too large, or has an invalid extension"));
        }
        let path = self.paths.artifacts.join(format!("{}.{extension}", content_id(bytes)));
        if !path.exists() {
            atomic(&path, bytes)?;
        }
        Ok(path)
    }
    pub fn bytes(&self, id: &str) -> Result<Vec<u8>> {
        if !valid(id) || !id.starts_with('h') {
            return Err(Error::new(Code::Artifact, "unknown host artifact"));
        }
        let path = self.paths.artifacts.join(id);
        let meta = fs::symlink_metadata(&path).map_err(|_| Error::new(Code::Artifact, "artifact not found"))?;
        if !meta.file_type().is_file() || meta.file_type().is_symlink() || meta.len() > MAX_ARTIFACT as u64 {
            return Err(Error::new(Code::Bounds, "artifact is too large or not a regular file"));
        }
        Ok(fs::read(path)?)
    }
}

fn content_id(bytes: &[u8]) -> String {
    format!("h{}", Sha256::digest(bytes)[..12].iter().map(|b| format!("{b:02x}")).collect::<String>())
}

pub fn valid(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stored_bytes_round_trip_and_ids_cannot_escape() {
        let d = tempfile::tempdir().unwrap();
        let a = Artifacts::new(Paths::at(d.path().to_path_buf()).unwrap());
        let id = a.put(&vec![7; crate::api::MAX_FRAME + 1]).unwrap();
        assert_eq!(a.bytes(&id).unwrap().len(), crate::api::MAX_FRAME + 1);
        assert!(!valid("../x") && a.bytes("../x").is_err());
    }
}
