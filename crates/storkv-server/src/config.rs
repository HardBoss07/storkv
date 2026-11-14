use std::path::PathBuf;

pub struct ServerConfig {
    pub data_dir: PathBuf,
    pub listen_addr: Option<String>,
    pub admin_addr: Option<String>,
}
