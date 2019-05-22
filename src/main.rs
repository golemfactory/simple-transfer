use actix_web::{post, web, App, HttpResponse, HttpServer, Responder};
use futures::{future, prelude::*};
use std::alloc::System;
use std::net::IpAddr;
use std::path::PathBuf;
use structopt::StructOpt;
use actix::Addr;
use crate::database::DatabaseManager;

mod command;
pub(crate) mod database;
pub(crate) mod error;

/// Simple resource transfer server for Golem Brass Network.
#[derive(StructOpt)]
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
    db : Addr<DatabaseManager>
}

impl State {

    fn id(&self) -> impl Future<Item = HttpResponse, Error = actix_web::error::Error> {
        database::id(&self.db).and_then(|id| {
            let version = env!("CARGO_PKG_VERSION").into();
            Ok(HttpResponse::Ok().json(command::IdResult { id, version }))
        }).map_err(|e| actix_web::error::ErrorInternalServerError(e))
    }


}

#[post("/api")]
fn api(
    state : web::Data<State>,
    body: web::Json<command::Command>,
) -> Box<dyn Future<Item = HttpResponse, Error = actix_web::error::Error>> {
    eprintln!("command={:?}", body.0);
    match body.0 {
        command::Command::Id => Box::new(state.id()),
        command::Command::Addresses => Box::new({
            future::ok(HttpResponse::Ok().json(command::AddressesResult {
                addresses: command::AddressSpec::TCP {
                    address: "0.0.0.0".into(),
                    port: 3282,
                },
            }))
        }),
        command::Command::Upload { files, timeout } => {
            
        }
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

    HttpServer::new(move || App::new().data(State { db: db.clone() }).service(api))
        .bind("127.0.0.1:3292")?
        .run()
}
