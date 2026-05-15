use std::path::PathBuf;

use anyhow::Result;

pub struct Storage {
    root: PathBuf,
}

impl Storage {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn tarball_path(&self, name: &str, version: &str) -> PathBuf {
        self.root
            .join(name)
            .join(version)
            .join(format!("{name}-{version}.tar.gz"))
    }

    pub async fn save(&self, name: &str, version: &str, data: &[u8]) -> Result<()> {
        let path = self.tarball_path(name, version);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&path, data).await?;
        Ok(())
    }

    pub async fn read(&self, name: &str, version: &str) -> Result<Vec<u8>> {
        Ok(tokio::fs::read(self.tarball_path(name, version)).await?)
    }

    pub fn exists(&self, name: &str, version: &str) -> bool {
        self.tarball_path(name, version).exists()
    }
}
