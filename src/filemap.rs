use serde::{Deserialize, Serialize};
use sha2::digest::Digest;
use std::borrow::Borrow;
use std::cmp::min;
use std::convert::TryInto;
use std::io::Read;
use std::path::Path;
use std::{fs, io};

pub const BLOCK_SIZE: usize = 1024 * 1024 * 4;

#[derive(Serialize, Deserialize, Clone)]
pub struct FileMap {
    pub file_name: String,
    pub file_size: u64,
    pub blocks: Vec<u128>,
}

#[derive(Serialize, Deserialize)]
pub struct BlobDesc {
    pub map_hash: u128,
    pub files: Vec<FileMap>,
}

#[inline]
fn extract_results<D: Digest>(digest: D) -> u128 {
    u128::from_le_bytes(digest.result()[0..16].try_into().unwrap())
}

pub fn hash_file(
    path: impl AsRef<Path>,
    file_name: impl Into<String>,
) -> Result<FileMap, io::Error> {
    let mut file = fs::OpenOptions::new().read(true).open(path)?;
    let file_size = file.metadata()?.len();
    let file_name = file_name.into();
    let num_of_blocks = ((file_size + BLOCK_SIZE as u64 - 1) / BLOCK_SIZE as u64)
        .try_into()
        .unwrap();

    let mut buf = Vec::with_capacity(BLOCK_SIZE);
    buf.resize(BLOCK_SIZE, 0);

    let mut blocks = Vec::with_capacity(num_of_blocks);

    // hash block

    let mut rem_file_bytes = file_size;
    while rem_file_bytes > 0 {
        let mut rem_block_bytes = BLOCK_SIZE;
        let mut digest = sha2::Sha224::new();
        while rem_block_bytes > 0 && rem_file_bytes > 0 {
            let to_read = min(buf.len(), rem_block_bytes);
            let len = file.read(&mut buf[..to_read])?;

            if len == 0 {
                return Err(io::Error::new(io::ErrorKind::Other, "Unexpected EOF"));
            }

            rem_block_bytes -= len;
            rem_file_bytes -= len as u64;
            digest.input(&buf[0..len]);
        }
        let hash = extract_results(digest);

        blocks.push(hash);
    }

    Ok(FileMap {
        file_name,
        file_size,
        blocks,
    })
}

pub fn hash_bundles(maps: impl IntoIterator<Item = impl Borrow<FileMap>>) -> u128 {
    let mut digest = sha2::Sha224::new();
    for map in maps {
        // TODO: Handle this
        bincode::serialize_into(&mut digest, map.borrow()).unwrap();
    }
    extract_results(digest)
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn test_hash_file() {
        let m = hash_file(
            "/home/prekucki/Downloads/swift-5.0-RELEASE-ubuntu18.04.tar.gz",
            "test_task_1.tar.gz",
        )
        .unwrap();

        for block in &m.blocks {
            eprintln!("block: {:032x}", block);
        }
        eprintln!("hash={:032x}", hash_bundles(Some(m)));
    }
}
