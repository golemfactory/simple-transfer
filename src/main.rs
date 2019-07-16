use crate::codec::{hash_to_hex, Block, GetBlock};
use crate::command::{DownloadResult, PeerInfo, UploadResult};
use crate::database::{DatabaseManager, RegisterHash};
use crate::download::find_peer;
use crate::filemap::{hash_block, FileMap};
use actix::Addr;
use actix_web::middleware::Logger;
use actix_web::{post, web, App, HttpResponse, HttpServer};
use futures::{future, prelude::*};

use flexi_logger::Duplicate;
use log::Level;
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::net::{self, IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use structopt::StructOpt;
use tokio_tcp::TcpListener;

mod codec;
mod command;
mod connection;
pub(crate) mod database;
mod download;
pub(crate) mod error;
pub(crate) mod filemap;
mod server;
mod version;

/// Simple resource transfer server for Golem Brass Network.
#[derive(StructOpt, Clone)]
#[structopt(raw(global_setting = "structopt::clap::AppSettings::DisableVersion"))]
struct ServerOpts {
    /// Database path
    #[structopt(long)]
    db: Option<PathBuf>,

    /// IP address to listen on
    #[structopt(long, default_value = "0.0.0.0", parse(try_from_str = "resolve_host"))]
    host: IpAddr,

    /// TCP port to listen on
    #[structopt(long, default_value = "3282")]
    port: u16,

    /// IP address for RPC to listen on
    #[structopt(
        long,
        default_value = "127.0.0.1",
        parse(try_from_str = "resolve_host")
    )]
    rpc_host: IpAddr,

    /// TCP port for RPC to listen on
    #[structopt(long, default_value = "3292")]
    rpc_port: u16,

    /// Database sweep interval in seconds
    #[structopt(long, default_value = "86400")]
    sweep_interval: u32,

    /// Database lifetime of shares in seconds
    #[structopt(long, default_value = "86400")]
    sweep_lifetime: u32,

    /// Log to file
    #[structopt(long)]
    logfile: Option<PathBuf>,

    /// Set the default logging level
    #[structopt(long, default_value = "info")]
    loglevel: log::Level,

    /// Prints version information
    #[structopt(long, short)]
    version: bool,
}

struct State {
    db: Addr<DatabaseManager>,
    opts: Arc<ServerOpts>,
}

fn resolve_host(src: &str) -> Result<IpAddr, <IpAddr as FromStr>::Err> {
    match src {
        "localhost" => Ok(Ipv4Addr::LOCALHOST.into()),
        _ => src.parse(),
    }
}

impl State {
    fn id(&self) -> impl Future<Item = HttpResponse, Error = actix_web::error::Error> {
        database::id(&self.db)
            .and_then(|id| {
                let id = crate::codec::hash_to_hex(id);
                let version = version::PACKAGE_VERSION.into();
                Ok(HttpResponse::Ok().json(command::IdResult { id, version }))
            })
            .map_err(|e| actix_web::error::ErrorInternalServerError(e))
    }

    fn addresses(&self) -> impl Future<Item = HttpResponse, Error = actix_web::error::Error> {
        future::ok(HttpResponse::Ok().json(command::AddressesResult {
            addresses: command::AddressSpec::TCP {
                address: self.opts.host.to_string(),
                port: self.opts.port,
            },
        }))
    }

    fn upload(
        &self,
        files: impl IntoIterator<Item = (PathBuf, String)>,
        _timeout: Option<f64>,
    ) -> impl Future<Item = HttpResponse, Error = actix_web::error::Error> {
        let hashed: Result<Vec<(filemap::FileMap, PathBuf)>, _> = files
            .into_iter()
            .map(|(path, file_name)| Ok((filemap::hash_file(&path, file_name)?, path)))
            .collect();

        let db = self.db.clone();

        hashed.into_future().and_then(move |file_maps| {
            let inline_data = if file_maps.len() == 1 {
                if file_maps[0].0.file_size < 200 {
                    match std::fs::read(&file_maps[0].1) {
                        Ok(v) => v,
                        Err(e) => return future::Either::B(future::err(e.into())),
                    }
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };

            // We do not trust timeout value for now.
            // Keeping file hash for 3 days should be good enough.
            let valid_to = Some(SystemTime::now() + Duration::from_secs(3600 * 24 * 3));

            future::Either::A(
                db.send(RegisterHash {
                    files: file_maps,
                    valid_to,
                    inline_data,
                })
                .then(|r| match r {
                    Err(_e) => Err(actix_web::error::ErrorInternalServerError("database lost")),
                    Ok(Err(e)) => Err(actix_web::error::ErrorInternalServerError(e)),
                    Ok(Ok(hash)) => Ok(HttpResponse::Ok().json(UploadResult {
                        hash: hash_to_hex(hash),
                    })),
                }),
            )
        })
    }

    fn check(
        &self,
        hash: &str,
    ) -> impl Future<Item = HttpResponse, Error = actix_web::error::Error> {
        let db = self.db.clone();
        u128::from_str_radix(hash, 16)
            .into_future()
            .map_err(|_e| actix_web::error::ErrorBadRequest("hash not found"))
            .and_then(move |hash| {
                db.send(database::GetHash(hash))
                    .flatten()
                    .map_err(|e| actix_web::error::ErrorInternalServerError(e))
            })
            .and_then(|r: Option<Arc<database::FileDesc>>| {
                if let Some(desc) = r {
                    Ok(HttpResponse::Ok().json(UploadResult {
                        hash: hash_to_hex(desc.map_hash),
                    }))
                } else {
                    Err(actix_web::error::ErrorBadRequest("hash not found"))
                }
            })
    }

    fn download(
        &self,
        hash: String,
        dest: PathBuf,
        peers: Vec<PeerInfo>,
        _timeout: Option<f64>,
    ) -> impl Future<Item = HttpResponse, Error = actix_web::error::Error> {
        let hash = match u128::from_str_radix(&hash, 16) {
            Err(e) => return future::Either::B(future::err(actix_web::error::ErrorBadRequest(e))),
            Ok(hash) => hash,
        };

        let peers: HashSet<_> = peers
            .into_iter()
            .map(|peer_info| match peer_info {
                PeerInfo::TCP(address, port) => SocketAddr::new(address.parse().unwrap(), port),
            })
            .collect();

        future::Either::A(
            find_peer(hash, self.db.clone(), peers.into_iter().collect())
                .and_then(move |(connection, file_map): (_, Vec<FileMap>)| {
                    use futures::prelude::*;

                    futures::stream::iter_ok(file_map.into_iter().enumerate())
                        .and_then(move |(file_no, file_map)| {
                            let hash = hash;
                            let out_path = dest.join(&file_map.file_name);
                            let connection = connection.clone();

                            if out_path.exists() {
                                log::warn!("path: {} already exists", out_path.display());
                                let _ = std::fs::rename(&out_path, out_path.with_extension("bak"));
                            }

                            std::fs::OpenOptions::new()
                                .write(true)
                                .create_new(true)
                                .open(&out_path)
                                .into_future()
                                .from_err()
                                .and_then(move |mut out_file| {
                                    futures::stream::iter_ok(
                                        file_map.blocks.into_iter().enumerate(),
                                    )
                                    .and_then(move |(block_no, block_hash_val)| {
                                        connection
                                            .send(GetBlock {
                                                hash,
                                                file_nr: file_no as u32,
                                                block_nr: block_no as u32,
                                            })
                                            // min 110Kb/s
                                            .timeout(Duration::from_secs(300))
                                            .flatten()
                                            .and_then(move |b| {
                                                let block_hash_calc =
                                                    hash_block(b.bytes.as_slice());
                                                if block_hash_calc == block_hash_val {
                                                    Ok(b)
                                                } else {
                                                    Err(crate::error::Error::InvalidBlockHash(
                                                        block_hash_calc,
                                                    ))
                                                }
                                            })
                                    })
                                    .for_each(move |b: Block| {
                                        out_file.write_all(b.bytes.as_slice())?;
                                        Ok(())
                                    })
                                    .and_then(|()| Ok(out_path))
                                })
                        })
                        .collect()
                        .and_then(|files| {
                            Ok(HttpResponse::Ok().json(DownloadResult { files: files }))
                        })
                })
                .map_err(|e| actix_web::error::ErrorInternalServerError(e)),
        )
    }

    fn mimic_download(
        &self,
        hash: String,
        dest: PathBuf,
    ) -> impl Future<Item = HttpResponse, Error = actix_web::error::Error> {
        let hash = match u128::from_str_radix(&hash, 16) {
            Err(e) => return future::Either::B(future::err(actix_web::error::ErrorBadRequest(e))),
            Ok(hash) => hash,
        };

        let db = self.db.clone();
        future::Either::A(
            db.send(database::GetHash(hash))
                .flatten()
                .map_err(|e| actix_web::error::ErrorInternalServerError(e))
                .and_then(|o: Option<Arc<database::FileDesc>>| {
                    o.ok_or(actix_web::error::ErrorBadRequest("hash not found"))
                        .into_future()
                        .from_err()
                        .and_then(|desc| {
                            futures::stream::iter_ok(desc.files.to_vec().into_iter().enumerate())
                                .and_then(move |(_, (file_map, path_buf))| {
                                    let out_path = dest.join(&file_map.file_name);

                                    if let Some(parent) = out_path.parent() {
                                        // Copy fails either way if the parent path does not exist
                                        let _ = fs::create_dir_all(parent);
                                    }

                                    fs::copy(path_buf, out_path.clone())
                                        .into_future()
                                        .map_err(|e| actix_web::error::ErrorInternalServerError(e))
                                        .and_then(|_| Ok(out_path))
                                })
                                .collect()
                        })
                        .and_then(|files| Ok(HttpResponse::Ok().json(DownloadResult { files })))
                })
                .map_err(|e| actix_web::error::ErrorInternalServerError(e)),
        )
    }
}

#[post("/api")]
fn api(
    state: web::Data<State>,
    body: web::Json<command::Command>,
) -> Box<dyn Future<Item = HttpResponse, Error = actix_web::error::Error>> {
    eprintln!("command={:?}", body.0);
    match body.0 {
        command::Command::Id => Box::new(state.id()),
        command::Command::Addresses => Box::new(state.addresses()),
        command::Command::Upload {
            files: Some(files),
            timeout,
            hash: None,
        } => Box::new(state.upload(files, timeout)),
        command::Command::Upload {
            files: None,
            timeout,
            hash: Some(hash),
        } => Box::new(state.check(&hash)),
        command::Command::Download {
            hash,
            dest,
            peers,
            timeout,
        } => {
            if peers.len() == 0 {
                // Legacy HyperG behaviour:
                // If no peers were provided, mimic the download process by copying locally stored files
                Box::new(state.mimic_download(hash, dest))
            } else {
                Box::new(state.download(hash, dest, peers, timeout))
            }
        }
        other_command => {
            log::warn!("bad command: {:?}", other_command);
            Box::new(future::err(actix_web::error::ErrorBadRequest(format!(
                "invalid command"
            ))))
        }
    }
}

fn log_string_for_level(level: &Level) -> &'static str {
    match level {
        Level::Error => "error",
        Level::Info => "info",
        Level::Debug => "hyperg=debug,info",
        Level::Trace => "hyperg=trace,info",
        Level::Warn => "hyperg=info,warn",
    }
}

fn is_dir_path(p: &Path) -> bool {
    p.to_str()
        .and_then(|s| s.chars().rev().next())
        .map(|ch| ch == std::path::MAIN_SEPARATOR)
        .unwrap_or(false)
}

fn detailed_format(
    w: &mut dyn std::io::Write,
    now: &mut flexi_logger::DeferredNow,
    record: &flexi_logger::Record,
) -> Result<(), std::io::Error> {
    write!(
        w,
        "{} {} {} {}",
        now.now().format("%Y-%m-%d %H:%M:%S"),
        record.level(),
        record.module_path().unwrap_or("<unnamed>"),
        &record.args()
    )
}

fn main() -> std::io::Result<()> {
    let args = ServerOpts::from_args();

    if args.version {
        println!("{}", version::PACKAGE_VERSION);
        return Ok(());
    }

    let log_builder = flexi_logger::Logger::with_env_or_str(log_string_for_level(&args.loglevel));

    if let Some(logfile) = &args.logfile {
        let logfile = logfile as &Path;

        let log_builder = if is_dir_path(logfile) {
            log_builder.directory(logfile)
        } else {
            match (logfile.file_name(), logfile.parent()) {
                (Some(_file_name), Some(dir_name)) if dir_name.is_dir() => {
                    log_builder.directory(dir_name).create_symlink(logfile)
                }
                _ => log_builder.create_symlink(logfile),
            }
        };
        log_builder
            .log_to_file()
            .duplicate_to_stderr(Duplicate::Info)
            .format_for_files(detailed_format)
            .start()
            .unwrap_or_else(|e| {
                eprintln!("Error {}", e);
                // fallback to stderr only logger.
                flexi_logger::Logger::with_env_or_str(log_string_for_level(&args.loglevel))
                    .start()
                    .unwrap()
            });
    } else {
        log_builder.start().unwrap();
    }

    version::startup_log();

    let sys = actix::System::new("hyperg");

    let db = database::database_manager(&args.db);
    let opts = Arc::new(args);

    let addr = net::SocketAddr::from((opts.host, opts.port));
    let listener = TcpListener::bind(&addr)?;

    let server_opts = opts.clone();
    let _transfer_server = server::Server::new(db.clone(), listener);

    let _rpc_server = HttpServer::new(move || {
        /*
        let listener =
            TcpListener::from_std(listener.try_clone().unwrap(), &Handle::default()).unwrap();
        */
        App::new()
            .wrap(Logger::default())
            .data(State {
                db: db.clone(),
                opts: opts.clone(),
            })
            .service(api)
    })
    .bind((server_opts.rpc_host, server_opts.rpc_port))?
    .start();

    sys.run()
}
