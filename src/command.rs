use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "command")]
#[serde(rename_all = "lowercase")]
pub enum Command {
    Id,
    Addresses,
    Upload {
        files: Option<HashMap<PathBuf, String>>,
        timeout: Option<f64>,
        hash: Option<String>,
        #[serde(default)]
        user: Option<User>,
    },
    Download {
        hash: String,
        dest: PathBuf,
        peers: Vec<PeerInfo>,
        timeout: Option<f64>,
        #[serde(default)]
        user: Option<User>,
    },
}

impl Command {
    pub fn log_start(&self) {
        match self {
            Command::Id => log::info!("command st ID"),
            Command::Addresses => log::info!("command st ADDRESSES"),
            Command::Upload {
                files,
                timeout,
                hash,
                user,
            } => log::info!(
                "command UPLOAD files={:?} timeout={:?} hash={:?} user={:?}",
                files,
                timeout,
                hash,
                user
            ),
            Command::Download {
                hash,
                dest,
                peers,
                timeout,
                user,
            } => log::info!(
                "command DOWNLOAD hash={}, dest={} peers={:?} timeout={:?} user={:?}",
                hash,
                dest.display(),
                peers,
                timeout,
                user
            ),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum AppEnv {
    TestNet,
    MainNet,
}

#[cfg(feature = "with-sentry")]
impl AppEnv {
    pub fn to_str(&self) -> std::borrow::Cow<'static, str> {
        match self {
            AppEnv::TestNet => std::borrow::Cow::Borrowed("testnet"),
            AppEnv::MainNet => std::borrow::Cow::Borrowed("mainnet"),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: String,
    pub env: AppEnv,
    #[serde(default)]
    pub node_name: Option<String>,
    #[serde(default)]
    pub golem_version: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum PeerInfo {
    TCP(String, u16),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IdResult {
    pub id: String,
    pub version: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct AddressesResult {
    pub addresses: AddressSpec,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum AddressSpec {
    TCP { address: String, port: u16 },
}

#[derive(Serialize, Deserialize, Debug)]
pub struct UploadResult {
    pub hash: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DownloadResult {
    pub files: Vec<PathBuf>,
}

#[cfg(test)]
mod test {
    use super::*;
    use serde_json;

    #[test]
    fn test_de() {
        let id_cmd_json = r#"{"command": "id"}"#;
        let id_cmd: Command = serde_json::from_str(id_cmd_json).unwrap();
        eprintln!("id_cmd={:?}", id_cmd);
        let upload_json = r#"{"command": "upload", "id": null, "files": {"/home/prekucki/.local/share/golem/default/rinkeby/ComputerRes/e339a264-71a9-11e9-b4e5-b6178fcd50f4/resources/e339a264-71a9-11e9-b4e5-b6178fcd50f4": "e339a264-71a9-11e9-b4e5-b6178fcd50f4"}, "timeout": null}"#;
        let upload_cmd: Command = serde_json::from_str(upload_json).unwrap();
        eprintln!("upload_cmd={:?}", upload_cmd);
        let download_json = r#"{"command": "download", "hash": "c0ceff522b00eccb95c43b43af67c9585c3d914642339f770800dd164d8b42cc", "dest": "/home/prekucki/.local/share/golem/default/rinkeby/ComputerRes/nonce/tmp", "peers": [{"TCP": ["10.30.10.219", 3282]}, {"TCP": ["10.30.10.219", 3282]}, {"TCP": ["5.226.70.53", 3282]}, {"TCP": ["172.17.0.1", 3282]}], "size": null, "timeout": null}"#;
        let download_cmd: Command = serde_json::from_str(download_json).unwrap();
        eprintln!("upload_cmd={:?}", download_cmd);
    }

}
