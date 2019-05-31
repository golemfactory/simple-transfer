use crate::codec::{hash_to_hex, Block, GetBlock};
use crate::command::{DownloadResult, PeerInfo, UploadResult};
use crate::connection::Connection;
use crate::database::{DatabaseManager, RegisterHash};
use crate::download::find_peer;
use crate::filemap::FileMap;
use actix::Addr;
use actix_web::middleware::Logger;
use actix_web::{post, web, App, HttpResponse, HttpServer};
use futures::{future, prelude::*};

use std::collections::HashSet;
use std::io::Write;
use std::net::{self, IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use structopt::StructOpt;
use tokio_reactor::Handle;
use tokio_tcp::TcpListener;

mod codec;
mod command;
mod connection;
pub(crate) mod database;
mod download;
pub(crate) mod error;
pub(crate) mod filemap;
mod server;

/// Simple resource transfer server for Golem Brass Network.
#[derive(StructOpt, Clone)]
struct ServerOpts {
    /// Database path
    #[structopt(long)]
    db: Option<PathBuf>,

    /// IP address to listen on
    #[structopt(long, default_value = "0.0.0.0")]
    host: IpAddr,

    /// TCP port to listen on
    #[structopt(long, default_value = "3282")]
    port: u16,

    /// IP address for RPC to listen on
    #[structopt(long, default_value = "127.0.0.1")]
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
    loglevel: String,
}

struct State {
    db: Addr<DatabaseManager>,
    opts: Arc<ServerOpts>,
}

impl State {
    fn id(&self) -> impl Future<Item = HttpResponse, Error = actix_web::error::Error> {
        database::id(&self.db)
            .and_then(|id| {
                let id = crate::codec::hash_to_hex(id);
                let version = env!("CARGO_PKG_VERSION").into();
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
    ) -> impl Future<Item = HttpResponse, Error = actix_web::error::Error> {
        let hashed: Result<Vec<(filemap::FileMap, PathBuf)>, _> = files
            .into_iter()
            .map(|(path, file_name)| Ok((filemap::hash_file(&path, file_name)?, path)))
            .collect();

        let db = self.db.clone();

        hashed.into_future().and_then(move |file_maps| {
            db.send(RegisterHash(file_maps)).then(|r| match r {
                Err(_e) => Err(actix_web::error::ErrorInternalServerError("database lost")),
                Ok(Err(e)) => Err(actix_web::error::ErrorInternalServerError(e)),
                Ok(Ok(hash)) => Ok(HttpResponse::Ok().json(UploadResult {
                    hash: hash_to_hex(hash),
                })),
            })
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
            .and_then(|r: Option<database::FileDesc>| {
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
        eprintln!("parsing hash={}", hash);
        let hash = u128::from_str_radix(&hash, 16).unwrap();
        let peers: HashSet<_> = peers
            .into_iter()
            .map(|peer_info| match peer_info {
                PeerInfo::TCP(address, port) => SocketAddr::new(address.parse().unwrap(), port),
            })
            .collect();

        find_peer(hash, self.db.clone(), peers.into_iter().collect())
            .and_then(
                move |(connection, file_map): (Addr<Connection>, Vec<FileMap>)| {
                    use futures::prelude::*;

                    futures::stream::iter_ok(file_map.into_iter().enumerate())
                        .and_then(move |(file_no, file_map)| {
                            let file_name = file_map.file_name;
                            let hash = hash;
                            let out_path = dest.join(file_name);
                            let connection = connection.clone();

                            let mut out_file = std::fs::OpenOptions::new()
                                .write(true)
                                .create_new(true)
                                .open(&out_path)
                                .unwrap();

                            futures::stream::iter_ok(file_map.blocks.into_iter().enumerate())
                                .and_then(move |(block_no, _block_hash)| {
                                    connection
                                        .send(GetBlock {
                                            hash,
                                            file_nr: file_no as u32,
                                            block_nr: block_no as u32,
                                        })
                                        .flatten()
                                })
                                .for_each(move |b: Block| {
                                    out_file.write_all(b.bytes.as_slice()).unwrap();
                                    Ok(())
                                })
                                .and_then(|()| Ok(out_path))
                        })
                        .collect()
                        .and_then(|files| {
                            Ok(HttpResponse::Ok().json(DownloadResult { files: files }))
                        })
                },
            )
            .map_err(|e| actix_web::error::ErrorInternalServerError(e))
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
        } => Box::new(state.upload(files)),
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
        } => Box::new(state.download(hash, dest, peers, timeout)),
        _ => unimplemented!(),
    }
}

fn main() -> std::io::Result<()> {
    let args = ServerOpts::from_args();
    flexi_logger::Logger::with_env_or_str("hyperg=debug,actix_web::middleware::logger=info")
        .start()
        .unwrap();

    let sys = actix::System::new("hyperg");

    let db = database::database_manager(&args.db);
    let opts = Arc::new(args);

    let addr = net::SocketAddr::from((opts.host, opts.port));
    let listener = Arc::new(net::TcpListener::bind(&addr)?);

    let server_opts = opts.clone();

    let _server = HttpServer::new(move || {
        let listener =
            TcpListener::from_std(listener.try_clone().unwrap(), &Handle::default()).unwrap();
        server::Server::new(db.clone(), listener);

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
