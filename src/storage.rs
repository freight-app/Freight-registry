use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use futures::StreamExt;
use object_store::{path::Path as ObjPath, ObjectStore};

// ── Backend ───────────────────────────────────────────────────────────────────

enum Backend {
    /// Local filesystem — original behaviour, lazily creates directories.
    Local(PathBuf),
    /// Any S3-compatible store (AWS, MinIO, …).
    S3(Arc<dyn ObjectStore>),
}

// ── Storage ───────────────────────────────────────────────────────────────────

pub struct Storage {
    backend: Backend,
}

impl Storage {
    /// Local-filesystem backend.  `root` need not exist yet.
    pub fn new(root: PathBuf) -> Self {
        Self { backend: Backend::Local(root) }
    }

    /// S3-compatible backend.
    ///
    /// `endpoint` — custom endpoint URL, e.g. `http://localhost:9000` for MinIO.
    ///   When the URL scheme is `http://` the connection is allowed to be plain-text.
    ///   Omit for real AWS (endpoint is derived from `region`).
    pub fn s3(
        bucket:   &str,
        endpoint: Option<&str>,
        key_id:   &str,
        secret:   &str,
        region:   &str,
    ) -> Result<Self> {
        use object_store::aws::AmazonS3Builder;
        let mut builder = AmazonS3Builder::new()
            .with_bucket_name(bucket)
            .with_access_key_id(key_id)
            .with_secret_access_key(secret)
            .with_region(region);
        if let Some(ep) = endpoint {
            let allow_http = ep.starts_with("http://");
            builder = builder.with_endpoint(ep).with_allow_http(allow_http);
        }
        Ok(Self { backend: Backend::S3(Arc::new(builder.build()?)) })
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn local_path(root: &PathBuf, name: &str, version: &str) -> PathBuf {
        root.join(name)
            .join(version)
            .join(format!("{name}-{version}.tar.gz"))
    }

    fn s3_key(name: &str, version: &str) -> ObjPath {
        ObjPath::from(format!("{name}/{version}/{name}-{version}.tar.gz"))
    }

    fn local_prebuilt_path(root: &PathBuf, name: &str, version: &str, triple: &str) -> PathBuf {
        root.join(name)
            .join(version)
            .join(format!("{name}-{version}-{triple}.tar.gz"))
    }

    fn s3_prebuilt_key(name: &str, version: &str, triple: &str) -> ObjPath {
        ObjPath::from(format!("{name}/{version}/{name}-{version}-{triple}.tar.gz"))
    }

    // ── Public API ────────────────────────────────────────────────────────────

    pub async fn save(&self, name: &str, version: &str, data: &[u8]) -> Result<()> {
        match &self.backend {
            Backend::Local(root) => {
                let path = Self::local_path(root, name, version);
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                tokio::fs::write(&path, data).await?;
            }
            Backend::S3(store) => {
                store
                    .put(&Self::s3_key(name, version), data.to_vec().into())
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn read(&self, name: &str, version: &str) -> Result<Vec<u8>> {
        match &self.backend {
            Backend::Local(root) => {
                Ok(tokio::fs::read(Self::local_path(root, name, version)).await?)
            }
            Backend::S3(store) => {
                let result = store.get(&Self::s3_key(name, version)).await?;
                Ok(result.bytes().await?.to_vec())
            }
        }
    }

    pub async fn save_prebuilt(&self, name: &str, version: &str, triple: &str, data: &[u8]) -> Result<()> {
        match &self.backend {
            Backend::Local(root) => {
                let path = Self::local_prebuilt_path(root, name, version, triple);
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                tokio::fs::write(&path, data).await?;
            }
            Backend::S3(store) => {
                store
                    .put(&Self::s3_prebuilt_key(name, version, triple), data.to_vec().into())
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn read_prebuilt(&self, name: &str, version: &str, triple: &str) -> Result<Vec<u8>> {
        match &self.backend {
            Backend::Local(root) => {
                Ok(tokio::fs::read(Self::local_prebuilt_path(root, name, version, triple)).await?)
            }
            Backend::S3(store) => {
                let result = store.get(&Self::s3_prebuilt_key(name, version, triple)).await?;
                Ok(result.bytes().await?.to_vec())
            }
        }
    }

    /// Remove all stored tarballs for a package. Silently succeeds if none exist.
    /// Store a README for a package. Stored once per package name (not per version).
    /// Existing README is overwritten so the latest publish wins.
    pub async fn save_readme(&self, name: &str, content: &[u8]) -> Result<()> {
        match &self.backend {
            Backend::Local(root) => {
                let path = root.join(name).join("README.md");
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                tokio::fs::write(path, content).await?;
            }
            Backend::S3(store) => {
                store
                    .put(&ObjPath::from(format!("{name}/README.md")), content.to_vec().into())
                    .await?;
            }
        }
        Ok(())
    }

    /// Read the stored README for a package. Returns `None` if not present.
    pub async fn read_readme(&self, name: &str) -> Option<String> {
        let bytes = match &self.backend {
            Backend::Local(root) => {
                tokio::fs::read(root.join(name).join("README.md")).await.ok()?
            }
            Backend::S3(store) => {
                store
                    .get(&ObjPath::from(format!("{name}/README.md")))
                    .await
                    .ok()?
                    .bytes()
                    .await
                    .ok()?
                    .to_vec()
            }
        };
        String::from_utf8(bytes).ok()
    }

    pub async fn delete_package_dir(&self, name: &str) -> Result<()> {
        match &self.backend {
            Backend::Local(root) => {
                let path = root.join(name);
                if path.exists() {
                    tokio::fs::remove_dir_all(path).await?;
                }
            }
            Backend::S3(store) => {
                let prefix = ObjPath::from(name.to_string());
                let mut stream = store.list(Some(&prefix));
                while let Some(result) = stream.next().await {
                    if let Ok(meta) = result {
                        let _ = store.delete(&meta.location).await;
                    }
                }
            }
        }
        Ok(())
    }
}
