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

    /// Remove all stored tarballs for a package. Silently succeeds if none exist.
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
