use crate::database::DatabaseManager;
use actix::prelude::*;
use actix_server::Io;
use actix_service::service_fn;

use std::{io, net};
use tokio_tcp::TcpStream;

pub fn new(
    db: Addr<DatabaseManager>,
    addr: impl net::ToSocketAddrs,
) -> io::Result<actix_server::Server> {
    Ok(actix_server::Server::build()
        .bind("gst", addr, move || {
            let db = db.clone();
            service_fn(move |stream: Io<TcpStream>| {
                let (tcp_stream, (), _) = stream.into_parts();
                let peer_addr = tcp_stream.peer_addr()?;
                log::info!("Connection from: {}", peer_addr);
                let _conn = crate::connection::Connection::new(db.clone(), tcp_stream, peer_addr);
                Ok::<_, io::Error>(())
            })
        })?
        .start())
}
