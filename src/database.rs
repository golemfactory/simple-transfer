use crate::error::Error;
use crate::filemap::FileMap;
use crate::user_report::UserReportHandle;
use actix::prelude::*;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use std::{fs, path, time};
use std::collections::hash_map::Entry;

/// metadata format
const FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Meta {
    /// Metadata format version
    format: u32,
    /// Node id
    id: u128,
    /// Reserved for future use
    flags: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct FileDesc {
    pub map_hash: u128,
    pub files: Vec<(FileMap, PathBuf)>,
    pub inline_data: Vec<u8>,
    pub valid_to: Option<time::SystemTime>,
}

impl FileDesc {
    #[inline]
    fn log_event(&self, event_name: &str) {
        for (_, file_path) in &self.files {
            log::info!(
                "{} {:032x} {}",
                event_name,
                self.map_hash,
                file_path.display()
            );
        }
    }
}

pub struct DatabaseManager {
    dir: PathBuf,
    id: Option<u128>,
    files: HashMap<u128, (Arc<FileDesc>, UserReportHandle)>,
}

impl DatabaseManager {
    fn load_hash(&mut self, p: &path::Path) -> Result<(), Error> {
        let desc: FileDesc = bincode::deserialize_from(fs::OpenOptions::new().read(true).open(p)?)?;
        desc.log_event("reshare");
        self.files
            .insert(desc.map_hash, (Arc::new(desc), UserReportHandle::empty()));
        Ok(())
    }

    fn init(&mut self) -> Result<(), Error> {
        let meta_path = self.dir.join("meta");
        let id: u128 = rand::thread_rng().gen();
        let meta = Meta {
            format: FORMAT_VERSION,
            id,
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

    fn remove_old_resources(&mut self) {
        let now = SystemTime::now();
        let expired_file_hashes: Vec<_> = self
            .files
            .iter()
            .filter(|(_, (v, _))| {
                v.valid_to
                    .as_ref()
                    .map(|valid_to| valid_to < &now)
                    .unwrap_or(false)
            })
            .map(|(&k, _)| k)
            .collect();

        for hash in expired_file_hashes {
            if let Some((file_desc, _)) = self.files.remove(&hash) {
                file_desc.log_event("unshare");
            }
        }
    }
}

impl Actor for DatabaseManager {
    type Context = SyncContext<Self>;

    fn started(&mut self, _: &mut Self::Context) {
        log::debug!("starting db on {}", self.dir.display());
        match self.load() {
            e @ Err(Error::InvalidMetaVersion { .. })
            | e @ Err(Error::MetadataNotFound)
            | e @ Err(Error::InvalidJsonFormat(_)) => {
                log::debug!("load meta error: {}", e.unwrap_err());
                // TODO: Better error handling.
                self.clear_dir().unwrap();
                self.init().unwrap();
            }
            Err(e) => {
                log::error!("init db fail: {}", e);
                System::current().stop()
            }
            Ok(()) => (),
        }
        log::info!("db started id=0x{:032x}", self.id.as_ref().unwrap());
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

    let addr = SyncArbiter::start(1, move || {
        let man = DatabaseManager {
            dir: dir.clone(),
            files: HashMap::new(),
            id: None,
        };

        man
    });
    let _ = GcWorker(addr.clone().recipient()).start();

    addr
}

struct GetId;

impl Message for GetId {
    type Result = Result<u128, Error>;
}

impl Handler<GetId> for DatabaseManager {
    type Result = Result<u128, Error>;

    fn handle(&mut self, _msg: GetId, _ctx: &mut Self::Context) -> Self::Result {
        self.id.clone().ok_or(Error::ServiceFail("DatabaseManager"))
    }
}

pub fn id(m: &Addr<DatabaseManager>) -> impl Future<Item = u128, Error = Error> {
    m.send(GetId).then(|r| match r {
        Ok(r) => r,
        Err(e) => Err(e.into()),
    })
}

pub struct GetHash(pub u128);

impl Message for GetHash {
    type Result = Result<Option<(Arc<FileDesc>, UserReportHandle)>, Error>;
}

impl Handler<GetHash> for DatabaseManager {
    type Result = Result<Option<(Arc<FileDesc>, UserReportHandle)>, Error>;

    fn handle(&mut self, msg: GetHash, _ctx: &mut Self::Context) -> Self::Result {
        if let Some((f, reporter)) = self.files.get(&msg.0) {
            Ok(Some((f.clone(), reporter.clone())))
        } else {
            Ok(None)
        }
    }
}

pub struct RemoveHash(pub u128);

impl Message for RemoveHash {
    type Result = Result<Option<Arc<FileDesc>>, Error>;
}

impl Handler<RemoveHash> for DatabaseManager {
    type Result = Result<Option<Arc<FileDesc>>, Error>;

    fn handle(&mut self, msg: RemoveHash, _ctx: &mut Self::Context) -> Self::Result {
        let prev = self.files.remove(&msg.0);
        Ok(if let Some((file_desc, _)) = prev {
            file_desc.log_event("unshare");
            Some(file_desc)
        } else {
            None
        })
    }
}

pub struct RegisterHash {
    pub files: Vec<(FileMap, PathBuf)>,
    pub valid_to: Option<time::SystemTime>,
    pub inline_data: Vec<u8>,
    pub reporter: UserReportHandle,
}

impl Message for RegisterHash {
    type Result = Result<u128, Error>;
}

impl Handler<RegisterHash> for DatabaseManager {
    type Result = Result<u128, Error>;

    fn handle(&mut self, msg: RegisterHash, _ctx: &mut Self::Context) -> Self::Result {
        let map_hash = crate::filemap::hash_bundles(msg.files.iter().map(|(map, _path)| map));
        let reporter = msg.reporter;
        let desc = Arc::new(FileDesc {
            map_hash,
            files: msg.files,
            inline_data: msg.inline_data,
            valid_to: msg.valid_to.clone(),
        });

        match self.files.entry(map_hash) {
            Entry::Occupied(mut ent) => {
                let prev_ent = ent.get_mut();
                let old_is_longer = match (prev_ent.0.valid_to, msg.valid_to) {
                    (None, _) => true,
                    (Some(prev_valid_to), Some(new_valid_to)) => prev_valid_to > new_valid_to,
                    _ => false,
                };
                if !old_is_longer {
                    prev_ent.0 = desc.clone();
                    desc.log_event("share extend");
                }
            }
            Entry::Vacant(ent) => {
                ent.insert((desc.clone(), reporter));
                desc.log_event("share");
            }
        }
        Ok(map_hash)
    }
}

#[derive(Default)]
pub struct List {}

impl Message for List {
    type Result = Vec<Arc<FileDesc>>;
}

impl Handler<List> for DatabaseManager {
    type Result = MessageResult<List>;

    fn handle(&mut self, _: List, _: &mut Self::Context) -> Self::Result {
        MessageResult(self.files.values().map(|(f, _)| f).cloned().collect())
    }
}

struct Gc;

impl Message for Gc {
    type Result = ();
}

impl Handler<Gc> for DatabaseManager {
    type Result = ();

    fn handle(&mut self, _: Gc, _: &mut Self::Context) -> Self::Result {
        self.remove_old_resources()
    }
}

struct GcWorker(Recipient<Gc>);

impl Actor for GcWorker {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        let _ = ctx.run_interval(Duration::from_secs(30), |act, ctx| {
            log::trace!("send gc start");
            match act.0.do_send(Gc) {
                Ok(()) => (),
                Err(e) => {
                    log::error!("gc error: {}", e);
                    ctx.stop()
                }
            }
        });
    }
}
