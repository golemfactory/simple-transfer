use crate::error::Error;
use actix::prelude::*;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::{fs, io, path};

/// metadata format
const FORMAT_VERSION: u32 = 1;

fn hash_to_hex(hash: u128) -> String {
    format!("{:032x}", hash)
}

#[derive(Serialize, Deserialize)]
struct Meta {
    /// Metadata format version
    format: u32,
    /// Node id
    id: String,
    /// Reserved for future use
    flags: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct FileMap {
    file_name: String,
    file_size: u64,
    blocks: Vec<u128>,
}

#[derive(Serialize, Deserialize)]
struct FileDesc {
    map_hash: u128,
    map: FileMap,
    file: PathBuf,
}

pub struct DatabaseManager {
    dir: PathBuf,
    id: Option<String>,
    files: HashMap<String, FileDesc>,
}

impl DatabaseManager {
    fn load_hash(&mut self, p: &path::Path) -> Result<(), Error> {
        let desc: FileDesc = bincode::deserialize_from(fs::OpenOptions::new().read(true).open(p)?)?;
        let hash_key = hash_to_hex(desc.map_hash);
        self.files.insert(hash_key, desc);
        Ok(())
    }

    fn init(&mut self) -> Result<(), Error> {
        let meta_path = self.dir.join("meta");
        let id: u128 = rand::thread_rng().gen();
        let meta = Meta {
            format: FORMAT_VERSION,
            id: hash_to_hex(id),
            flags: Vec::new(),
        };
        serde_json::to_writer_pretty(
            fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(meta_path)?,
            &meta,
        )?;
        self.id = Some(meta.id);
        Ok(())
    }

    fn load(&mut self) -> Result<(), Error> {
        let meta = self.dir.join("meta");
        if meta.exists() {
            let meta_def: Meta =
                serde_json::from_reader(fs::OpenOptions::new().read(true).open(meta)?)?;
            if meta_def.format != FORMAT_VERSION {
                return Err(Error::InvalidMetaVersion {
                    detected_version: meta_def.format,
                });
            }
            self.id = Some(meta_def.id)
        } else {
            return Err(Error::MetadataNotFound);
        }
        for entry in fs::read_dir(&self.dir)? {
            let path = entry?.path();
            if path.extension() == Some(".fhash".as_ref()) {
                if let Err(e) = self.load_hash(&path) {
                    log::error!("load hash error: {}", e);
                    fs::remove_file(path)?;
                }
            }
        }
        Ok(())
    }

    fn clear_dir(&mut self) -> Result<(), Error> {
        Ok(())
    }
}

impl Actor for DatabaseManager {
    type Context = SyncContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        log::debug!("starting db on {}", self.dir.display());
        match self.load() {
            e @ Err(Error::InvalidMetaVersion { .. })
            | e @ Err(Error::MetadataNotFound)
            | e @ Err(Error::InvalidJsonFormat(_)) => {
                log::debug!("load meta error: {}", e.unwrap_err());
                self.clear_dir();
                self.init();
            }
            Err(e) => {
                log::error!("init db fail: {}", e);
                System::current().stop()
            }
            Ok(()) => (),
        }
        log::info!("db started id={}", self.id.as_ref().unwrap())
    }
}

static APP_INFO: app_dirs::AppInfo = app_dirs::AppInfo {
    name: "hyperg",
    author: "golem.network",
};

pub fn database_manager(cache_path: &Option<PathBuf>) -> Addr<DatabaseManager> {
    let dir = cache_path.clone().unwrap_or_else(|| {
        app_dirs::app_dir(app_dirs::AppDataType::UserCache, &APP_INFO, "db").unwrap()
    });

    SyncArbiter::start(1, move || {
        let man = DatabaseManager {
            dir: dir.clone(),
            files: HashMap::new(),
            id: None,
        };

        man
    })
}

struct GetId;

impl Message for GetId {
    type Result = Result<String ,Error>;
}

impl Handler<GetId> for DatabaseManager {
    type Result = Result<String ,Error>;

    fn handle(&mut self, msg: GetId, ctx: &mut Self::Context) -> Self::Result {
        self.id.clone().ok_or(Error::ServiceFail("DatabaseManager"))
    }
}

pub fn id(m : &Addr<DatabaseManager>) -> impl Future<Item=String, Error=Error> {
    m.send(GetId).then(|r| match r {
        Ok(r) => r,
        Err(e) => Err(e.into()),
    })
}

