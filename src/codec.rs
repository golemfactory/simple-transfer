use crate::filemap::FileMap;
use bytes::{Buf, BufMut, ByteOrder, BytesMut, LittleEndian};
use rand::ErrorKind;
use serde::{Deserialize, Serialize};
use std::convert::{TryFrom, TryInto};
use std::fmt::Display;
use std::io;
use std::io::prelude::*;
use tokio_io::codec::{Decoder, Encoder};

const PROTO_VERSION: u8 = 1;

const MAX_PACKET_SIZE: usize = 1024 * 1024 * 8;

pub fn hash_to_hex(hash: u128) -> String {
    format!("{:032x}", hash)
}

#[repr(u8)]
pub enum Op {
    Nop = 0,
    Hello = 1,
    Ask = 2,
    AskReply = 3,
    GetBlock = 4,
    Block = 5,
}

pub enum StCommand {
    Nop,
    Hello(Hello),
    Ask(u128),
    AskReply(AskReply),
    GetBlock(GetBlock),
    Block(Block),
}

impl StCommand {
    pub fn hello(id: u128) -> StCommand {
        StCommand::Hello(Hello::new(id))
    }

    pub fn ask_reply(hash: u128, files: Option<Vec<FileMap>>) -> Self {
        StCommand::AskReply(AskReply { hash, files })
    }

    pub fn block(hash: u128, file_nr: u32, block_nr: u32, bytes: Vec<u8>) -> Self {
        StCommand::Block(Block {
            hash,
            block_nr,
            file_nr,
            bytes,
        })
    }

    pub fn display(&self) -> impl Display {
        match self {
            StCommand::Nop => format!("[nop]"),
            StCommand::Hello(h) => format!("[hello id:{}, v:{}", h.node_id, h.proto_version),
            StCommand::Ask(hash) => format!("[ask {}]", hash),
            StCommand::AskReply(hash) => format!("[ask-replay ...]"),
            StCommand::GetBlock(b) => format!(
                "[get-block hash:{}, file-no:{}, block-no:{}]",
                b.hash, b.file_nr, b.block_nr
            ),
            StCommand::Block(b) => format!(
                "[block hash:{}, file-no:{}, block-no:{}]",
                b.hash, b.file_nr, b.block_nr
            ),
        }
    }
}

impl StCommand {
    fn decode(op: Op, buf: &[u8]) -> Result<Self, bincode::Error> {
        Ok(match op {
            Op::Nop => StCommand::Nop,
            Op::Hello => StCommand::Hello(bincode::deserialize(buf)?),
            Op::Ask => StCommand::Ask(bincode::deserialize(buf)?),
            Op::AskReply => StCommand::AskReply(bincode::deserialize(buf)?),
            Op::GetBlock => StCommand::GetBlock(bincode::deserialize(buf)?),
            Op::Block => StCommand::Block(bincode::deserialize(buf)?),
        })
    }
}

impl Op {
    pub fn size(&self) -> Option<u32> {
        match self {
            Op::Nop => Some(0),
            Op::Hello => Some(17),
            Op::Ask => Some(16),
            Op::AskReply => None,
            Op::GetBlock => None,
            Op::Block => None,
        }
    }
}

impl TryFrom<u8> for Op {
    type Error = io::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Op::Nop),
            1 => Ok(Op::Hello),
            2 => Ok(Op::Ask),
            3 => Ok(Op::AskReply),
            4 => Ok(Op::GetBlock),
            5 => Ok(Op::Block),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unknown packet opcode",
            )),
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Hello {
    pub proto_version: u8,
    pub node_id: u128,
}

impl Hello {
    pub fn is_valid(&self) -> bool {
        self.proto_version == PROTO_VERSION
    }

    pub fn new(node_id: u128) -> Self {
        Hello {
            proto_version: PROTO_VERSION,
            node_id,
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Ask {
    pub hash: u128,
}

#[derive(Default, Serialize, Deserialize)]
pub struct AskReply {
    pub hash: u128,
    // None if unknown hash
    pub files: Option<Vec<FileMap>>,
}

#[derive(Default, Serialize, Deserialize)]
pub struct GetBlock {
    pub hash: u128,
    pub file_nr: u32,
    pub block_nr: u32,
}

#[derive(Default, Serialize, Deserialize, Clone)]
pub struct Block {
    pub hash: u128,
    pub block_nr: u32,
    pub file_nr: u32,
    pub bytes: Vec<u8>,
}

#[derive(Default)]
pub struct StCodec {}

impl Decoder for StCodec {
    type Item = StCommand;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 1 {
            return Ok(None);
        }

        let op_code: Op = src[0].try_into()?;
        let (size, prefix_size) = match op_code.size() {
            Some(v) => (v as usize, 0),
            None => {
                if src.len() < 5 {
                    return Ok(None);
                } else {
                    (LittleEndian::read_u32(&src[1..5]) as usize, 4)
                }
            }
        };
        if size > MAX_PACKET_SIZE {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "packet too big"));
        }

        if src.len() >= size + prefix_size + 1 {
            src.split_to(prefix_size + 1);
            let buf = src.split_to(size);
            Ok(Some(StCommand::decode(op_code, buf.as_ref()).map_err(
                |e| io::Error::new(io::ErrorKind::InvalidData, e),
            )?))
        } else {
            Ok(None)
        }
    }
}

fn put_into_buf<T: Serialize>(size: usize, buf: &mut BytesMut, item: &T) -> io::Result<()> {
    let mut w = buf.writer();
    bincode::serialize_into(&mut w, item).map_err(|e| {
        match e.as_ref() {
            bincode::ErrorKind::Io(io_err) => return io::Error::new(io_err.kind(), e),
            _ => (),
        };
        io::Error::new(io::ErrorKind::Other, e)
    })
}

impl Encoder for StCodec {
    type Item = StCommand;
    type Error = io::Error;

    fn encode(&mut self, msg: StCommand, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let (op, prefix_size, size) = match &msg {
            StCommand::Nop => (Op::Nop, 0usize, 0usize),
            StCommand::Hello(..) => (Op::Hello, 0, 17),
            StCommand::Ask(..) => (Op::Ask, 0, 16),
            StCommand::AskReply(reply) => (
                Op::AskReply,
                4,
                bincode::serialized_size(reply).unwrap() as usize,
            ),
            StCommand::GetBlock(get_block) => (
                Op::GetBlock,
                4,
                bincode::serialized_size(get_block).unwrap() as usize,
            ),
            StCommand::Block(block) => (
                Op::Block,
                4,
                bincode::serialized_size(block).unwrap() as usize,
            ),
        };
        dst.reserve(1 + prefix_size + size);

        dst.put_u8(op as u8);
        if prefix_size == 4 {
            dst.put_u32_le(size as u32);
        } else {
            assert_eq!(prefix_size, 0)
        }
        match msg {
            StCommand::Nop => Ok(()),
            StCommand::Hello(hello) => put_into_buf(size, dst, &hello),
            StCommand::Ask(ask) => put_into_buf(size, dst, &ask),
            StCommand::AskReply(ask_reply) => put_into_buf(size, dst, &ask_reply),
            StCommand::GetBlock(get_block) => put_into_buf(size, dst, &get_block),
            StCommand::Block(block) => put_into_buf(size, dst, &block),
        }
    }
}

#[cfg(test)]
mod test {

    use super::*;

    #[test]
    fn test_size() {
        let hello_size = bincode::serialized_size(&Hello::default()).unwrap() as u32;

        assert_eq!(hello_size, 17);

        let ask_size = bincode::serialized_size(&Ask::default()).unwrap() as u32;

        assert_eq!(ask_size, 16);
    }

    #[test]
    fn test_hello() {
        log::info!("start");
        let mut codec = StCodec::default();

        let mut buf = BytesMut::new();
        codec
            .encode(
                StCommand::Hello(Hello {
                    proto_version: 0,
                    node_id: 10,
                }),
                &mut buf,
            )
            .unwrap();

        let mut r = buf.take();
        let v = codec.decode(&mut r).unwrap().unwrap();
        match v {
            StCommand::Hello(Hello {
                proto_version,
                node_id,
            }) => {
                assert_eq!(proto_version, 0);
                assert_eq!(node_id, 10)
            }
            _ => assert!(false),
        }
    }

    #[test]
    fn test_block() {
        let mut codec = StCodec::default();
        let block = Block {
            hash: 0x1212deadbeef1212,
            file_nr: 0,
            block_nr: 0,
            bytes: vec![1, 2, 3, 4, 5, 6],
        };

        let mut buf = BytesMut::new();
        codec
            .encode(StCommand::Block(block.clone()), &mut buf)
            .unwrap();

        let mut r = buf.take();
        let v = codec.decode(&mut r).unwrap().unwrap();
        match v {
            StCommand::Block(Block {
                hash,
                file_nr,
                block_nr,
                bytes,
            }) => {
                assert_eq!(hash, block.hash);
                assert_eq!(block_nr, block.block_nr);
                assert_eq!(bytes, block.bytes);
            }
            _ => assert!(false),
        }
    }

}
