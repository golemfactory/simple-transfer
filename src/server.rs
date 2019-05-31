use crate::database::DatabaseManager;
use actix::prelude::*;

use std::net;
use tokio_tcp::{TcpListener, TcpStream};

pub struct Server {
    db: Addr<DatabaseManager>,
}

impl Server {
    pub fn new(db: Addr<DatabaseManager>, listener: TcpListener) -> Addr<Self> {
        Self::create(|ctx| {
            ctx.add_message_stream(listener.incoming().map_err(|_| ()).and_then(|st| {
                let addr = st
                    .peer_addr()
                    .map_err(|e| log::error!("get peer addr: {}", e))?;
                Ok(TcpConnect(st, addr))
            }));
            Server { db }
        })
    }
}

#[derive(Message)]
pub struct TcpConnect(pub TcpStream, pub net::SocketAddr);

impl Actor for Server {
    type Context = Context<Self>;

    fn started(&mut self, _ctx: &mut Self::Context) {
        // TODO
    }
}

impl Handler<TcpConnect> for Server {
    /// this is response for message, which is defined by `ResponseType` trait
    /// in this case we just return unit.
    type Result = ();

    fn handle(&mut self, msg: TcpConnect, _: &mut Context<Self>) {
        log::info!("Connection from: {}", msg.1);
        let _conn = crate::connection::Connection::new(self.db.clone(), msg.0, msg.1);
    }
}
