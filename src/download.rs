#![allow(unused_imports)]

use crate::codec::{Ask, AskReply};
use crate::connection::{Connection, ConnectionRef};
use crate::database::DatabaseManager;
use crate::error::Error;
use crate::filemap::FileMap;
use actix::prelude::*;
use futures::prelude::*;
use std::net;

use failure::_core::time::Duration;
use tokio_tcp::{ConnectFuture, TcpStream};

pub fn connect(
    db: Addr<DatabaseManager>,
    addr: net::SocketAddr,
    reporter: crate::user_report::UserReportHandle,
) -> impl Future<Item = ConnectionRef, Error = Error> {
    TcpStream::connect(&addr).from_err().and_then(move |c| {
        reporter.add_note(|| format!("connected to {}", addr));
        Connection::new_managed(db, c, addr, &reporter)
    })
}

pub fn find_peer(
    hash: u128,
    db: Addr<DatabaseManager>,
    addr: Vec<net::SocketAddr>,
    reporter: crate::user_report::UserReportHandle,
) -> impl Future<Item = (ConnectionRef, Vec<FileMap>, net::SocketAddr), Error = Error> {
    let connections = addr.into_iter().map(move |addr| {
        let hash = hash;
        let reporter = reporter.clone();

        reporter.add_note(|| format!("connecting to {}", addr));

        connect(db.clone(), addr, reporter.clone())
            .and_then(move |connection| {
                connection
                    .send(Ask::new(hash))
                    .flatten()
                    .and_then(move |reply: AskReply| match reply.files {
                        Some(files) => Ok((connection, files, addr)),
                        None => Err(Error::ResourceNotFound(reply.hash)),
                    })
            })
            .map_err(move |e| {
                reporter.add_err(|| format!("failed to connect to {}: {}", addr, e));

                e
            })
    });

    futures::select_ok(connections).and_then(|(v, _)| Ok(v))
}
