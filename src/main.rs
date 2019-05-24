use crate::database::DatabaseManager;
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

mod codec;
mod command;
pub(crate) mod database;
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
        let hashed: Result<Vec<filemap::FileMap>, _> = files
            .into_iter()
            .map(|(path, file_name)| filemap::hash_file(&path, file_name))
            .collect();

        future::ok(HttpResponse::Ok().finish())
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
    flexi_logger::Logger::with_env_or_str("hyperg=debug")
        .start()
        .unwrap();

    let sys = actix::System::new("hyperg");

    let db = database::database_manager(&args.db);
    let opts = Arc::new(args);

    let addr = net::SocketAddr::from((opts.host, opts.port));
    let listener = Arc::new(net::TcpListener::bind(&addr)?);

    let _server = HttpServer::new(move || {
        let listener =
            TcpListener::from_std(listener.try_clone().unwrap(), &Handle::default()).unwrap();
        server::Server::new(db.clone(), listener);

        App::new()
            .data(State {
                db: db.clone(),
                opts: opts.clone(),
            })
            .service(api)
    })
    .bind("127.0.0.1:3292")?
    .start();

    sys.run()
}
