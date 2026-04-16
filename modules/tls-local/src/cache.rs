use std::fs;
use std::path::PathBuf;

static LOCAL_CA_LOG_ONCE: std::sync::Once = std::sync::Once::new();

pub struct LocalTlsCache {
    path: PathBuf,
}

impl LocalTlsCache {
    pub fn new(path: PathBuf) -> Self {
        if !path.exists() {
            fs::create_dir_all(&path).ok();
        }
        LOCAL_CA_LOG_ONCE.call_once(|| {
            ferron_core::log_info!(
                "Local CA certificate can be found in \"{}\". Import the CA certificate into your \
            system trust store to trust the generated certificates.",
                path.join("ca.crt").display()
            );
        });
        Self { path }
    }

    pub fn get_ca_cert(&self) -> Option<String> {
        fs::read_to_string(self.path.join("ca.crt")).ok()
    }

    pub fn get_ca_key(&self) -> Option<String> {
        fs::read_to_string(self.path.join("ca.key")).ok()
    }

    pub fn save_ca(&self, cert: &str, key: &str) -> std::io::Result<()> {
        fs::write(self.path.join("ca.crt"), cert)?;
        fs::write(self.path.join("ca.key"), key)?;
        Ok(())
    }

    pub fn get_leaf_cert(&self, san_hash: &str) -> Option<String> {
        fs::read_to_string(self.path.join(format!("{}.crt", san_hash))).ok()
    }

    pub fn get_leaf_key(&self, san_hash: &str) -> Option<String> {
        fs::read_to_string(self.path.join(format!("{}.key", san_hash))).ok()
    }

    pub fn save_leaf(&self, san_hash: &str, cert: &str, key: &str) -> std::io::Result<()> {
        fs::write(self.path.join(format!("{}.crt", san_hash)), cert)?;
        fs::write(self.path.join(format!("{}.key", san_hash)), key)?;
        Ok(())
    }
}
