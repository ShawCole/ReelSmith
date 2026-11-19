//! Object storage. Completed source uploads cannot be deleted or replaced.
//! The local adapter is what `cargo run -p reelsmith-portal` uses. R2 presigning lives in `r2`.

use crate::error::Error;
use crate::ids::hex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const DEFAULT_PART_LIMIT: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadManifest {
    pub id: String,
    pub key: String,
    pub total_size: u64,
    pub role: String,
    pub parts: Vec<u32>,
    pub complete: bool,
    pub sha256: Option<String>,
}

#[derive(Clone, Debug)]
pub struct LocalStore {
    root: PathBuf,
    part_limit: u64,
}

impl LocalStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, Error> {
        let root = root.into();
        fs::create_dir_all(root.join("objects")).map_err(|_| Error::invalid("could not create storage"))?;
        fs::create_dir_all(root.join("uploads")).map_err(|_| Error::invalid("could not create storage"))?;
        Ok(Self { root, part_limit: DEFAULT_PART_LIMIT })
    }

    pub fn create_upload(&self, key: &str, total_size: u64, role: &str, max_bytes: u64) -> Result<UploadManifest, Error> {
        validate_key(key)?;
        if role != "aroll" && role != "broll" {
            return Err(Error::invalid("role must be aroll or broll"));
        }
        if total_size == 0 || total_size > max_bytes {
            return Err(Error::invalid("upload size is empty or larger than the configured limit"));
        }
        if self.object_path(key).exists() {
            return Err(Error::conflict("that source file already exists and cannot be replaced"));
        }
        let id = format!("up_{}", hex(&crate::ids::random_bytes(8)?));
        let manifest =
            UploadManifest { id: id.clone(), key: key.to_string(), total_size, role: role.to_string(), parts: Vec::new(), complete: false, sha256: None };
        self.write_manifest(&manifest)?;
        Ok(manifest)
    }

    pub fn write_part(&self, id: &str, part: u32, bytes: &[u8]) -> Result<UploadManifest, Error> {
        if part == 0 || bytes.is_empty() {
            return Err(Error::invalid("a part needs a number starting at 1 and a body"));
        }
        if bytes.len() as u64 > self.part_limit {
            return Err(Error::invalid("a part is larger than 8 MiB"));
        }
        let mut manifest = self.manifest(id)?;
        if manifest.complete {
            return Err(Error::conflict("this upload is already complete and the source cannot be changed"));
        }
        let dir = self.part_dir(&manifest.id);
        fs::create_dir_all(&dir)?;
        let path = dir.join(part.to_string());
        let mut file = File::create(&path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if !manifest.parts.contains(&part) {
            manifest.parts.push(part);
            manifest.parts.sort_unstable();
        }
        let received = self.received_bytes(&manifest)?;
        if received > manifest.total_size {
            let _ = fs::remove_file(path);
            return Err(Error::invalid("the parts are larger than the declared upload"));
        }
        self.write_manifest(&manifest)?;
        Ok(manifest)
    }

    pub fn status(&self, id: &str) -> Result<UploadManifest, Error> {
        self.manifest(id)
    }

    pub fn complete(&self, id: &str) -> Result<UploadManifest, Error> {
        let mut manifest = self.manifest(id)?;
        if manifest.complete {
            return Ok(manifest);
        }
        self.check_parts(&manifest)?;
        let final_path = self.object_path(&manifest.key);
        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = File::create(&final_path)?;
        let mut hasher = Sha256::new();
        let mut written = 0u64;
        for part in &manifest.parts {
            let mut input = File::open(self.part_dir(&manifest.id).join(part.to_string()))?;
            let mut buf = [0u8; 64 * 1024];
            loop {
                let n = input.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                output.write_all(&buf[..n])?;
                hasher.update(&buf[..n]);
                written = written.saturating_add(n as u64);
            }
        }
        output.sync_all()?;
        if written != manifest.total_size {
            let _ = fs::remove_file(&final_path);
            return Err(Error::invalid("the uploaded bytes do not match the declared size"));
        }
        manifest.complete = true;
        manifest.sha256 = Some(hex(&hasher.finalize()));
        self.write_manifest(&manifest)?;
        Ok(manifest)
    }

    /// Source files stay. This is the guard the style pipeline's "never modify a source" rule needs.
    pub fn delete_source(&self, key: &str) -> Result<(), Error> {
        validate_key(key)?;
        if self.object_path(key).exists() {
            return Err(Error::conflict("source uploads are kept; versions and backups never delete them"));
        }
        Err(Error::not_found("no source with that key"))
    }

    pub fn read(&self, key: &str) -> Result<Vec<u8>, Error> {
        validate_key(key)?;
        let path = self.object_path(key);
        if !path.exists() {
            return Err(Error::not_found("no object"));
        }
        fs::read(&path).map_err(|_| Error::invalid("could not read the object"))
    }

    /// Where a stored object lives on disk (the server session's path map reads it there).
    pub fn path_of(&self, key: &str) -> Result<PathBuf, Error> {
        validate_key(key)?;
        Ok(self.object_path(key))
    }

    fn check_parts(&self, manifest: &UploadManifest) -> Result<(), Error> {
        if manifest.parts.is_empty() {
            return Err(Error::invalid("the upload has no parts"));
        }
        for (index, part) in manifest.parts.iter().enumerate() {
            let expected = u32::try_from(index).map_err(|_| Error::invalid("too many parts"))?.saturating_add(1);
            if *part != expected {
                return Err(Error::invalid("a part is missing; resume from the first gap"));
            }
        }
        let received = self.received_bytes(manifest)?;
        if received != manifest.total_size {
            return Err(Error::invalid("the upload is not finished"));
        }
        Ok(())
    }

    fn received_bytes(&self, manifest: &UploadManifest) -> Result<u64, Error> {
        let mut total = 0u64;
        for part in &manifest.parts {
            let meta = fs::metadata(self.part_dir(&manifest.id).join(part.to_string()))?;
            total = total.saturating_add(meta.len());
        }
        Ok(total)
    }

    fn manifest(&self, id: &str) -> Result<UploadManifest, Error> {
        validate_id(id)?;
        let path = self.root.join("uploads").join(format!("{id}.json"));
        let text = fs::read_to_string(&path).map_err(|_| Error::not_found("no upload"))?;
        serde_json::from_str(&text).map_err(|_| Error::invalid("upload record is unreadable"))
    }

    fn write_manifest(&self, manifest: &UploadManifest) -> Result<(), Error> {
        validate_id(&manifest.id)?;
        let path = self.root.join("uploads").join(format!("{}.json", manifest.id));
        atomic_write(&path, &serde_json::to_vec_pretty(manifest).map_err(|_| Error::invalid("could not record the upload"))?)
    }

    fn part_dir(&self, id: &str) -> PathBuf {
        self.root.join("uploads").join(id)
    }

    fn object_path(&self, key: &str) -> PathBuf {
        self.root.join("objects").join(key)
    }
}

pub fn validate_key(key: &str) -> Result<(), Error> {
    if key.is_empty() || key.len() > 512 || key.starts_with('/') || key.contains("..") || key.contains('\\') || key.contains('\0') {
        return Err(Error::invalid("storage key is not allowed"));
    }
    if !key.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-')) {
        return Err(Error::invalid("storage key has a character that is not allowed"));
    }
    Ok(())
}

pub fn validate_id(id: &str) -> Result<(), Error> {
    if id.is_empty() || id.len() > 80 || !id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') {
        return Err(Error::invalid("upload id is not allowed"));
    }
    Ok(())
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn sanitize_filename(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars().take(80) {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_' {
            out.push(ch);
        } else if ch == ' ' {
            out.push('_');
        }
    }
    if out.is_empty() { "upload.bin".to_string() } else { out }
}
