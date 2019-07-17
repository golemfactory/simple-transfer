use crate::codec::{AskReply, Block, GetBlock, StCodec, StCommand};

use crate::database;
use crate::database::{DatabaseManager, FileDesc};
use crate::error::{Error, ProtocolError};
use crate::filemap::{FileMap, BLOCK_SIZE};
use actix::io::WriteHandler;
use actix::prelude::*;
use actix::{Actor, Addr, Context};

use futures::unsync::oneshot;
use std::cmp::min;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::ops::Deref;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::{io, net};
use tokio_codec::FramedRead;
use tokio_io::io::WriteHalf;
use tokio_io::AsyncRead;
use tokio_tcp::TcpStream;

static CONNECTION_IDS: AtomicUsize = AtomicUsize::new(0);

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);

pub struct Connection {
    connection_id: usize,
    db: Addr<DatabaseManager>,
    peer_addr: net::SocketAddr,
    framed: actix::io::FramedWrite<WriteHalf<TcpStream>, StCodec>,
    peer_id: Option<u128>,
    current_file: Option<Arc<database::FileDesc>>,
    block_requests: HashMap<GetBlock, oneshot::Sender<Result<Block, Error>>>,
    ask_requests: HashMap<u128, oneshot::Sender<Result<AskReply, Error>>>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        log::debug!(
            "closed connection id={}, peer={}",
            self.connection_id,
            self.peer_addr
        );
    }
}

impl Actor for Connection {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        log::info!(
            "opened connection [{}] [{}]",
            self.connection_id,
            self.peer_addr
        );
        ctx.run_later(HANDSHAKE_TIMEOUT, |act, ctx| {
            if act.peer_id.is_none() {
                log::error!("identification timeout for {}", act.peer_addr);
                act.close_with_error(ProtocolError::HandshakeTimeout, ctx)
            }
        });
    }

    fn stopped(&mut self, _: &mut Self::Context) {
        log::info!(
            "closed connection [{}] [{}]",
            self.connection_id,
            self.peer_addr
        );
    }
}

impl Connection {
    fn new_addr(
        db: Addr<DatabaseManager>,
        tcp_stream: TcpStream,
        peer_addr: net::SocketAddr,
    ) -> Addr<Connection> {
        let connection_id = CONNECTION_IDS.fetch_add(1, Ordering::SeqCst);
        let addr: Addr<Connection> = Connection::create(move |ctx| {
            let (r, w) = tcp_stream.split();
            let framed = actix::io::FramedWrite::new(w, StCodec::default(), ctx);
            log::debug!("opened connection id={}, peer={}", connection_id, peer_addr);

            Connection::add_stream(FramedRead::new(r, StCodec::default()), ctx);
            Connection {
                connection_id,
                db,
                framed,
                peer_addr,
                peer_id: None,
                current_file: None,
                block_requests: HashMap::new(),
                ask_requests: HashMap::new(),
            }
        });

        addr
    }

    pub fn new(
        db: Addr<DatabaseManager>,
        tcp_stream: TcpStream,
        peer_addr: net::SocketAddr,
    ) -> impl Future<Item = Addr<Connection>, Error = Error> {
        let id_fut = database::id(&db);
        let addr = Self::new_addr(db, tcp_stream, peer_addr);

        id_fut.and_then(move |id| {
            addr.send(crate::codec::Hello::new(id))
                .flatten()
                .and_then(move |()| Ok(addr))
        })
    }

    pub fn new_managed(
        db: Addr<DatabaseManager>,
        tcp_stream: TcpStream,
        peer_addr: net::SocketAddr,
    ) -> impl Future<Item = ConnectionRef, Error = Error> {
        let id_fut = database::id(&db);
        let addr = ConnectionRef(Self::new_addr(db, tcp_stream, peer_addr));

        id_fut.and_then(move |id| {
            addr.send(crate::codec::Hello::new(id))
                .flatten()
                .and_then(move |()| Ok(addr))
        })
    }

    fn send_ask_reply(&mut self, file_desc: FileDesc, _ctx: &mut <Self as Actor>::Context) {
        let reply = StCommand::ask_reply(
            file_desc.map_hash,
            Some(
                file_desc
                    .files
                    .into_iter()
                    .map(|(file_map, _path)| file_map)
                    .collect(),
            ),
        );

        self.framed.write(reply)
    }

    fn send_ask_reply_not_found(&mut self, hash: u128, _ctx: &mut <Self as Actor>::Context) {
        self.framed.write(StCommand::ask_reply(hash, None))
    }

    fn handle_ask(&mut self, hash: u128, ctx: &mut <Self as Actor>::Context) {
        if let Some(file_desc) = self.current_file.clone() {
            if file_desc.map_hash == hash {
                return self.send_ask_reply(file_desc.as_ref().clone(), ctx);
            }
        }

        let reply_hash = hash;

        let f = self
            .db
            .send(database::GetHash(hash))
            .then(|v| match v {
                Err(e) => Err(e.into()),
                Ok(v) => v,
            })
            .into_actor(self)
            .and_then(
                move |file_desc: Option<Arc<FileDesc>>, act: &mut Self, ctx| match file_desc {
                    Some(file_desc) => {
                        if file_desc.map_hash == reply_hash {
                            act.current_file = Some(file_desc.clone());
                            act.send_ask_reply(file_desc.as_ref().clone(), ctx);
                            fut::ok(())
                        } else {
                            panic!("unexpected result on db call")
                        }
                    }
                    None => {
                        act.send_ask_reply_not_found(reply_hash, ctx);
                        fut::ok(())
                    }
                },
            )
            .map_err(|_e, act, ctx| {
                log::error!("fail to handle ask from: {}", &act.peer_addr);
                ctx.stop()
            });

        ctx.spawn(f);
    }

    // TODO: return error in proto
    fn handle_get_block(&mut self, get_block: GetBlock, ctx: &mut <Self as Actor>::Context) {
        let file_map = match &self.current_file {
            Some(v) if v.map_hash == get_block.hash => v,
            Some(_) => {
                log::error!("wrong hash before get_block");
                ctx.stop();
                return;
            }
            None => {
                log::error!("get hash before get_block needed");
                ctx.stop();
                return;
            }
        };

        if file_map.inline_data.len() > 0 && get_block.file_nr == 0 && get_block.block_nr == 0 {
            self.framed.write(StCommand::block(
                get_block.hash,
                get_block.file_nr,
                get_block.block_nr,
                file_map.inline_data.clone(),
            ));
            return;
        }

        let (map, path) = match file_map.files.get(get_block.file_nr as usize) {
            Some((ref map, ref path)) => (map, path),
            None => {
                log::error!(
                    "invalid file_no: {} for {}",
                    get_block.file_nr,
                    get_block.hash
                );
                ctx.stop();
                return;
            }
        };
        let bytes = match read_block(path, map, get_block.block_nr) {
            Err(e) => {
                log::error!("read fail: {}", e);
                ctx.stop();
                return;
            }
            Ok(bytes) => bytes,
        };

        self.framed.write(StCommand::block(
            get_block.hash,
            get_block.file_nr,
            get_block.block_nr,
            bytes,
        ));
    }

    fn handle_block(&mut self, b: Block, _ctx: &mut <Self as Actor>::Context) {
        let get_block = GetBlock {
            hash: b.hash,
            file_nr: b.file_nr,
            block_nr: b.block_nr,
        };
        if let Some(r) = self.block_requests.remove(&get_block) {
            let _ = r.send(Ok(b));
        } else {
            log::error!("response for not requested block");
        }
    }

    fn handle_ask_reply(&mut self, b: AskReply, _ctx: &mut <Self as Actor>::Context) {
        if let Some(h) = self.ask_requests.remove(&b.hash) {
            let _ = h.send(Ok(b));
        } else {
            log::warn!("unexpected ask reply");
        }
    }

    fn close_with_error(&mut self, e: ProtocolError, ctx: &mut <Self as Actor>::Context) {
        std::mem::replace(&mut self.block_requests, HashMap::new())
            .into_iter()
            .for_each(|(_, sender)| {
                let _ = sender.send(Err(e.into_err()));
            });
        std::mem::replace(&mut self.ask_requests, HashMap::new())
            .into_iter()
            .for_each(|(_, sender)| {
                let _ = sender.send(Err(e.into_err()));
            });
        self.framed.close();
        ctx.run_later(Duration::from_millis(10), |_, ctx| {
            ctx.stop();
        });
        //
    }
}

fn read_block(
    path: impl AsRef<Path>,
    file_map: &FileMap,
    block_no: u32,
) -> Result<Vec<u8>, io::Error> {
    log::debug!(
        "read block for: [{}], block_no={}, file_name={}",
        path.as_ref().display(),
        block_no,
        file_map.file_name
    );
    let offset = block_no as u64 * BLOCK_SIZE as u64;
    if file_map.file_size < offset {
        return Err(io::Error::new(ErrorKind::Other, "invalid offset"));
    }
    let size = min(file_map.file_size - offset, BLOCK_SIZE as u64) as usize;
    let mut file = OpenOptions::new().read(true).open(path)?;
    file.seek(SeekFrom::Start(offset))?;

    let mut bytes_vec = Vec::with_capacity(size);
    bytes_vec.resize(size, 0);

    let mut bytes = bytes_vec.as_mut_slice();
    while bytes.len() > 0 {
        let n = file.read(bytes)?;
        if n == 0 {
            return Err(io::Error::new(
                ErrorKind::UnexpectedEof,
                "unexpected end of file",
            ));
        }
        bytes = &mut bytes[n..];
    }
    Ok(bytes_vec)
}

impl StreamHandler<StCommand, io::Error> for Connection {
    fn handle(&mut self, item: StCommand, ctx: &mut Self::Context) {
        log::debug!("incomming packet={}", item.display());
        match item {
            StCommand::Nop => (),
            StCommand::Bye => {
                log::info!("disconnect from: {}", self.peer_addr);
                self.close_with_error(ProtocolError::Disconnect, ctx)
            }
            StCommand::Hello(h) => {
                if h.is_valid() {
                    self.peer_id = Some(h.node_id);
                } else {
                    log::error!("invalid handshake from: {}", self.peer_addr);
                    self.close_with_error(ProtocolError::InvalidHandshake, ctx)
                }
            }
            StCommand::Ask(hash) => {
                if self.peer_id.is_none() {
                    log::error!("ask without handshake, disconnect");
                    self.close_with_error(ProtocolError::MissingHandshake, ctx)
                } else {
                    self.handle_ask(hash, ctx)
                }
            }
            StCommand::AskReply(r) => self.handle_ask_reply(r, ctx),
            StCommand::GetBlock(b) => self.handle_get_block(b, ctx),
            StCommand::Block(b) => self.handle_block(b, ctx),
        }
    }
}

impl WriteHandler<io::Error> for Connection {}

impl Handler<crate::codec::Ask> for Connection {
    type Result = ActorResponse<Self, AskReply, Error>;

    fn handle(&mut self, msg: crate::codec::Ask, _ctx: &mut Self::Context) -> Self::Result {
        let (rx, tx) = oneshot::channel();
        if let Some(_prev) = self.ask_requests.insert(msg.hash, rx) {
            log::error!("duplicate ask");
        } else {
            self.framed.write(StCommand::Ask(msg.hash))
        }
        ActorResponse::r#async(tx.flatten().into_actor(self))
    }
}

impl Handler<crate::codec::GetBlock> for Connection {
    type Result = ActorResponse<Self, Block, Error>;

    fn handle(&mut self, msg: GetBlock, _ctx: &mut Self::Context) -> Self::Result {
        let (rx, tx) = oneshot::channel();
        if let Some(_prev) = self.block_requests.insert(msg.clone(), rx) {
            log::error!("duplicate get");
        } else {
            self.framed.write(StCommand::GetBlock(msg))
        }
        ActorResponse::r#async(tx.flatten().into_actor(self))
    }
}

impl Handler<crate::codec::Hello> for Connection {
    type Result = Result<(), Error>;

    fn handle(&mut self, msg: crate::codec::Hello, _ctx: &mut Self::Context) -> Self::Result {
        self.framed.write(StCommand::hello(msg.node_id));
        Ok(())
    }
}

impl Handler<crate::codec::Bye> for Connection {
    type Result = Result<(), Error>;

    fn handle(
        &mut self,
        _msg: crate::codec::Bye,
        ctx: &mut <Self as Actor>::Context,
    ) -> Self::Result {
        self.framed.write(StCommand::Bye);
        log::info!("bye to: {}", self.peer_addr);

        ctx.run_later(Duration::from_secs(5), |act, ctx| {
            act.close_with_error(ProtocolError::DisconnectByMe, ctx)
        });
        Ok(())
    }
}

pub struct ConnectionRef(Addr<Connection>);

impl Deref for ConnectionRef {
    type Target = Addr<Connection>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for ConnectionRef {
    fn drop(&mut self) {
        self.0.do_send(crate::codec::Bye::new());
    }
}
