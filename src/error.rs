use failure::Fail;
use std::io;

#[derive(Debug, Fail)]
pub enum Error {
    #[fail(display = "{}", _0)]
    IO(#[cause] io::Error),
    #[fail(display = "invalid format: {}", _0)]
    InvalidJsonFormat(#[cause] serde_json::Error),
    #[fail(display = "invalid format: {}", _0)]
    InvalidBinFormat(#[cause] bincode::Error),
    #[fail(display = "invalid matadata version: {}", detected_version)]
    InvalidMetaVersion { detected_version: u32 },
    #[fail(display = "matadata not found")]
    MetadataNotFound,
    #[fail(display = "{} not working", _0)]
    ServiceFail(&'static str),
    #[fail(display = "{}", _0)]
    Mailbox(actix::MailboxError),
}

macro_rules! convert {
    {
        $($t:path => $opt:ident),*
    } => {
        $(impl From<$t> for Error {
            fn from(e : $t) -> Self {
                Error::$opt(e)
            }
        })*
    };
}

convert! {
    io::Error => IO,
    bincode::Error => InvalidBinFormat,
    serde_json::Error => InvalidJsonFormat,
    actix::MailboxError => Mailbox
}
