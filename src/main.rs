use actix_web::{web, App, HttpServer, Responder, post, HttpResponse};
use futures::{future, prelude::*};

mod command;

#[post("/api")]
fn api(body : web::Json<command::Command>) -> impl Future<Item=HttpResponse, Error=actix_web::error::Error> {
    eprintln!("command={:?}", body.0);
    match body.0 {
        command::Command::Id => {
            let id = "123".into();
            let version = "0.3.0".into();

            future::ok(HttpResponse::Ok().json(command::IdResult {
                id, version
            }))
        }
        command::Command::Addresses => {
            future::ok(HttpResponse::Ok().json(command::AddressesResult {
                addresses: command::AddressSpec::TCP {
                    address: "0.0.0.0".into(),
                    port: 3282
                }
            }))
        }
        _ => unimplemented!()
    }
}



fn main() -> std::io::Result<()> {
    HttpServer::new(|| App::new().service(api))
        .bind("127.0.0.1:3292")?
        .run()
}
