use crate::codec::Op::Hello;
use crate::codec::{AskReply, Block, GetBlock, Op, StCodec, StCommand};
use crate::command::Command;
use crate::database;
use crate::database::{DatabaseManager, FileDesc};
use crate::error::Error;
use crate::filemap::{FileMap, BLOCK_SIZE};
use actix::io::WriteHandler;
use actix::prelude::*;
use actix::{Actor, Addr, Context};
use futures::task::AtomicTask;
use futures::unsync::oneshot;
use std::cmp::min;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::{io, net};
use tokio_codec::FramedRead;
use tokio_io::io::WriteHalf;
use tokio_io::AsyncRead;
use tokio_tcp::TcpStream;

pub struct Connection {
    db: Addr<DatabaseManager>,
    peer_addr: net::SocketAddr,
    framed: actix::io::FramedWrite<WriteHalf<TcpStream>, StCodec>,
    peer_id: Option<u128>,
    current_file: Option<database::FileDesc>,
    block_requests: HashMap<GetBlock, oneshot::Sender<Block>>,
    ask_requests: HashMap<u128, oneshot::Sender<AskReply>>,
}

impl Actor for Connection {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        let f = self.send_hello(ctx).map_err(|e, _, ctx| {
            log::error!("failed to get id: {}", e);
            ctx.stop()
        });
        ctx.spawn(f);
    }
}

impl Connection {
    pub fn new(
        db: Addr<DatabaseManager>,
        tcp_stream: TcpStream,
        peer_addr: net::SocketAddr,
    ) -> Addr<Connection> {
        Connection::create(move |ctx| {
            let (r, w) = tcp_stream.split();
            let framed = actix::io::FramedWrite::new(w, StCodec::default(), ctx);
            Connection::add_stream(FramedRead::new(r, StCodec::default()), ctx);
            Connection {
                db,
                framed,
                peer_addr,
                peer_id: None,
                current_file: None,
                block_requests: HashMap::new(),
                ask_requests: HashMap::new(),
            }
        })
    }

    fn send_hello(
        &mut self,
        ctx: &mut <Self as Actor>::Context,
    ) -> impl ActorFuture<Actor = Self, Item = (), Error = Error> {
        database::id(&self.db)
            .into_actor(self)
            .and_then(|id, act: &mut Connection, ctx| {
                fut::ok(act.framed.write(StCommand::hello(id)))
            })
    }

    fn send_ask_reply(&mut self, file_desc: FileDesc, ctx: &mut <Self as Actor>::Context) {
        let reply = StCommand::ask_reply(
            file_desc.map_hash,
            Some(
                file_desc
                    .files
                    .into_iter()
                    .map(|(file_map, path)| file_map)
                    .collect(),
            ),
        );

        self.framed.write(reply)
    }

    fn send_ask_reply_not_found(&mut self, hash: u128, ctx: &mut <Self as Actor>::Context) {
        self.framed.write(StCommand::ask_reply(hash, None))
    }

    fn handle_ask(&mut self, hash: u128, ctx: &mut <Self as Actor>::Context) {
        if let Some(file_desc) = self.current_file.clone() {
            if file_desc.map_hash == hash {
                return self.send_ask_reply(file_desc, ctx);
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
                move |file_desc: Option<FileDesc>, mut act, ctx| match file_desc {
                    Some(file_desc) => {
                        if file_desc.map_hash == reply_hash {
                            act.current_file = Some(file_desc.clone());
                            act.send_ask_reply(file_desc, ctx);
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
            .map_err(|e, act, ctx| {
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

    fn handle_block(&mut self, b : Block, ctx: &mut <Self as Actor>::Context) {
        let get_block = GetBlock {
            hash: b.hash,
            file_nr: b.file_nr,
            block_nr: b.block_nr
        };
        if let Some(r) = self.block_requests.remove(&get_block) {
            let _ = r.send(b);
        }
        else {
            log::error!("response for not requested block");
        }
    }
}

fn read_block(
    path: impl AsRef<Path>,
    file_map: &FileMap,
    block_no: u32,
) -> Result<Vec<u8>, io::Error> {
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
        match item {
            StCommand::Hello(h) => self.peer_id = Some(h.node_id),
            StCommand::Ask(hash) => {
                if self.peer_id.is_none() {
                    log::error!("ask without handshake, disconnect");
                    ctx.stop()
                } else {
                    self.handle_ask(hash, ctx)
                }
            }
            StCommand::GetBlock(b) => self.handle_get_block(b, ctx),
            StCommand::Block(b) => self.handle_block(b ,ctx),
            p => {
                log::error!("unexpected packet from: {}", self.peer_addr);
                ctx.stop()
            }
        }
    }
}

impl WriteHandler<io::Error> for Connection {}

impl Handler<crate::codec::Ask> for Connection {
    type Result = ActorResponse<Self, AskReply, Error>;

    fn handle(&mut self, msg: crate::codec::Ask, ctx: &mut Self::Context) -> Self::Result {
        let (rx, tx) = oneshot::channel();
        if let Some(prev) = self.ask_requests.insert(msg.hash, rx) {
            log::error!("duplicate ask");
        } else {
            self.framed.write(StCommand::Ask(msg.hash))
        }
        ActorResponse::r#async(tx.from_err().into_actor(self))
    }
}

impl Handler<crate::codec::GetBlock> for Connection {
    type Result = ActorResponse<Self, Block, Error>;

    fn handle(&mut self, msg: GetBlock, ctx: &mut Self::Context) -> Self::Result {
        let (rx, tx) = oneshot::channel();
        if let Some(prev) = self.block_requests.insert(msg.clone(), rx) {
            log::error!("duplicate get");
        }
        else {
            self.framed.write(StCommand::GetBlock(msg))
        }
        ActorResponse::r#async(tx.from_err().into_actor(self))
    }
}