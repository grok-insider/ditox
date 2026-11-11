use std::fs::{self, File, OpenOptions};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Write};
use std::path::Path;

pub fn write_image_blob(
    image_dir: &str,
    hash: &str,
    ext: &str,
    bytes: &[u8],
) -> io::Result<String> {
    let shard = hash
        .get(..2)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "hash is too short"))?;
    let dir_path = format!("{image_dir}/{shard}");
    fs::create_dir_all(&dir_path)?;

    let final_path = format!("{dir_path}/{hash}.{ext}");
    let tmp_path = format!("{final_path}.tmp-{:x}", unique_suffix());
    let written = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp_path, &final_path)
    })();
    if let Err(err) = written {
        delete_file(&tmp_path);
        return Err(err);
    }
    if let Ok(dir) = File::open(&dir_path) {
        let _ = dir.sync_all();
    }
    Ok(final_path)
}

pub fn delete_file(path: &str) {
    let _ = fs::remove_file(path);
}

pub fn file_exists(path: &str) -> bool {
    Path::new(path).exists()
}

fn unique_suffix() -> u64 {
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u32(std::process::id());
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blob_is_written_sharded_by_its_hash_and_leaves_no_temporary_file() {
        let root = crate::testing::TempDir::new("blob");
        let image_dir = root.path("images-v2");
        let hash = "ab".to_string() + &"0".repeat(62);
        let path = write_image_blob(&image_dir, &hash, "png", b"bytes").unwrap();
        assert_eq!(path, format!("{image_dir}/ab/{hash}.png"));
        assert_eq!(fs::read(&path).unwrap(), b"bytes");
        assert_eq!(
            write_image_blob(&image_dir, &hash, "png", b"bytes").unwrap(),
            path
        );
        delete_file(&path);
        assert!(!file_exists(&path));
    }
}
