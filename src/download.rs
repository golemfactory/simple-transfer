#![allow(unused_imports)]

use crate::codec::{Ask, AskReply};
use crate::connection::{Connection, ConnectionRef};
use crate::database::DatabaseManager;
use crate::error::Error;
use crate::filemap::FileMap;
use actix::prelude::*;
use futures::prelude::*;
use std::net;

use tokio_tcp::TcpStream;

pub fn connect(
    db: Addr<DatabaseManager>,
    addr: net::SocketAddr,
) -> impl Future<Item = ConnectionRef, Error = Error> {
    TcpStream::connect(&addr)
        .from_err()
        .and_then(move |c| Connection::new_managed(db, c, addr))
}

pub fn find_peer(
    hash: u128,
    db: Addr<DatabaseManager>,
    addr: Vec<net::SocketAddr>,
) -> impl Future<Item = (ConnectionRef, Vec<FileMap>), Error = Error> {
    let connections = addr.into_iter().map(move |addr| {
        let hash = hash;
        connect(db.clone(), addr).and_then(move |connection| {
            connection
                .send(Ask::new(hash))
                .flatten()
                .and_then(move |reply: AskReply| match reply.files {
                    Some(files) => Ok((connection, files)),
                    None => Err(Error::ResourceNotFound(reply.hash)),
                })
        })
    });

    futures::select_ok(connections).and_then(|(v, _)| Ok(v))
}
