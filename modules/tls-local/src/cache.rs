use std::fs;
use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

pub struct LocalTlsCache {
    path: PathBuf,
}

impl LocalTlsCache {
    pub fn new(path: PathBuf) -> Self {
        if !path.exists() {
            fs::create_dir_all(&path).ok();
        }
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

        // Write CA private key with restrictive permissions (0o600 on Unix)
        let key_path = self.path.join("ca.key");
        let mut open_options = std::fs::OpenOptions::new();
        open_options.write(true).create(true).truncate(true);

        #[cfg(unix)]
        open_options.mode(0o600);

        let mut file = open_options.open(&key_path)?;
        std::io::Write::write_all(&mut file, key.as_bytes())?;

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

        // Write leaf private key with restrictive permissions (0o600 on Unix)
        let key_path = self.path.join(format!("{}.key", san_hash));
        let mut open_options = std::fs::OpenOptions::new();
        open_options.write(true).create(true).truncate(true);

        #[cfg(unix)]
        open_options.mode(0o600);

        let mut file = open_options.open(&key_path)?;
        std::io::Write::write_all(&mut file, key.as_bytes())?;

        Ok(())
    }

    pub fn ca_path(&self) -> PathBuf {
        self.path.join("ca.crt")
    }
}
