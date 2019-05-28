use crate::codec::hash_to_hex;
use crate::command::UploadResult;
use crate::database::{DatabaseManager, RegisterHash};
use actix::Addr;
use actix_web::{post, web, App, HttpResponse, HttpServer};
use futures::{future, prelude::*};
use std::alloc::System;
use std::net::{self, IpAddr};
use std::path::PathBuf;
use std::sync::Arc;
use structopt::StructOpt;
use tokio_reactor::Handle;
use tokio_tcp::TcpListener;
use actix_web::middleware::Logger;

mod codec;
mod command;
mod connection;
pub(crate) mod database;
mod download;
pub(crate) mod error;
pub(crate) mod filemap;
mod flatten;
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
                Err(e) => Err(actix_web::error::ErrorInternalServerError("database lost")),
                Ok(Err(e)) => Err(actix_web::error::ErrorInternalServerError(e)),
                Ok(Ok(hash)) => Ok(HttpResponse::Ok().json(UploadResult {
                    hash: hash_to_hex(hash),
                })),
            })
        })
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
        command::Command::Upload { files, timeout } => Box::new(state.upload(files)),
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
    .bind((server_opts.rpc_addr, server_opts.rpc_port))?
    .start();

    sys.run()
}
