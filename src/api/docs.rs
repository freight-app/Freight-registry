use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{ApiError, ApiResult};
use crate::{auth::AuthToken, db::DEFAULT_CHANNEL, AppState};

#[derive(Debug, Deserialize)]
enum DocLanguage {
    C,
    Cpp,
    Rust,
    Fortran,
    D,
    Ada,
    Java,
    Go,
    Zig,
    Kotlin,
    Swift,
    Python,
    TypeScript,
    JavaScript,
    CSharp,
    Php,
    Ruby,
    Lua,
    R,
    Haskell,
    Unknown,
}

impl DocLanguage {
    fn label(&self) -> &'static str {
        match self {
            Self::C => "C",
            Self::Cpp => "C++",
            Self::Rust => "Rust",
            Self::Fortran => "Fortran",
            Self::D => "D",
            Self::Ada => "Ada",
            Self::Java => "Java",
            Self::Go => "Go",
            Self::Zig => "Zig",
            Self::Kotlin => "Kotlin",
            Self::Swift => "Swift",
            Self::Python => "Python",
            Self::TypeScript => "TypeScript",
            Self::JavaScript => "JavaScript",
            Self::CSharp => "C#",
            Self::Php => "PHP",
            Self::Ruby => "Ruby",
            Self::Lua => "Lua",
            Self::R => "R",
            Self::Haskell => "Haskell",
            Self::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Deserialize)]
enum DocKind {
    Function,
    Struct,
    Class,
    Enum,
    Typedef,
    Variable,
    Macro,
    Module,
    Subroutine,
    Interface,
    Unknown,
}

impl DocKind {
    fn label(&self) -> &'static str {
        match self {
            Self::Function => "fn",
            Self::Struct => "struct",
            Self::Class => "class",
            Self::Enum => "enum",
            Self::Typedef => "type",
            Self::Variable => "var",
            Self::Macro => "macro",
            Self::Module => "mod",
            Self::Subroutine => "sub",
            Self::Interface => "iface",
            Self::Unknown => "item",
        }
    }
}

#[derive(Debug, Deserialize)]
enum TagKind {
    Brief,
    Param,
    Return,
    Note,
    See,
    Since,
    Deprecated,
    Example,
    Warning,
    Other(String),
}

impl TagKind {
    fn label(&self) -> &str {
        match self {
            Self::Brief => "Brief",
            Self::Param => "Parameter",
            Self::Return => "Returns",
            Self::Note => "Note",
            Self::See => "See also",
            Self::Since => "Since",
            Self::Deprecated => "Deprecated",
            Self::Example => "Example",
            Self::Warning => "Warning",
            Self::Other(s) => s.as_str(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct DocTag {
    kind: TagKind,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    text: String,
}

#[derive(Debug, Deserialize)]
enum Access {
    Public,
    Protected,
    Private,
}

impl Access {
    fn label(&self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Protected => "protected",
            Self::Private => "private",
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct DocMeta {
    #[serde(default)]
    template_params: Vec<String>,
    #[serde(default)]
    access: Option<Access>,
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    attrs: Vec<String>,
    #[serde(default)]
    group: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DocItem {
    #[serde(default)]
    name: String,
    kind: DocKind,
    #[serde(default)]
    brief: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    tags: Vec<DocTag>,
    file: std::path::PathBuf,
    #[serde(default)]
    line: usize,
    lang: DocLanguage,
    #[serde(default)]
    signature: String,
    #[serde(default)]
    meta: DocMeta,
}

/// PUT /api/v1/packages/:name/:version/docs
///
/// Body: raw msgpack bytes (output of `docify dump`).
/// Requires the caller to be an owner of the package.
pub async fn put_docs(
    auth: AuthToken,
    State(state): State<Arc<AppState>>,
    Path((name, version)): Path<(String, String)>,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    if body.is_empty() {
        return Err(ApiError::bad_request("docs body is empty"));
    }
    // Validate that it is parseable docify MessagePack without coupling this
    // standalone registry crate to the docify implementation crate.
    rmp_serde::from_slice::<Vec<DocItem>>(&body)
        .map_err(|_| ApiError::bad_request("invalid msgpack — expected docify dump output"))?;

    let (pkg, _versions) = state
        .db
        .get_package(&name, DEFAULT_CHANNEL)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("package `{name}` not found")))?;

    if !state
        .db
        .user_can_manage_package(&name, &pkg.channel, auth.user.id)
        .await?
    {
        return Err(ApiError::forbidden("you do not own this package"));
    }

    state
        .storage
        .save_docs(&name, &version, &body)
        .await
        .map_err(|e| ApiError::internal(format!("storage error: {e}")))?;

    state.db.audit(
        Some(auth.user.id),
        "upload_docs",
        Some(&name),
        Some(&version),
        None,
    );

    Ok(Json(json!({ "ok": true })))
}

/// GET /api/v1/packages/:name/:version/docs
///
/// Returns the docset as a JSON array of DocItem objects.
/// Returns 404 if no docs have been uploaded for this version.
pub async fn get_docs(
    State(state): State<Arc<AppState>>,
    Path((name, version)): Path<(String, String)>,
) -> impl IntoResponse {
    // Resolve "latest" alias
    let resolved_version = if version == "latest" {
        match state.db.get_package(&name, DEFAULT_CHANNEL).await {
            Ok(Some((pkg, _))) => pkg.latest_version.unwrap_or(version),
            _ => version,
        }
    } else {
        version
    };

    let blob = match state.storage.read_docs(&name, &resolved_version).await {
        Some(b) => b,
        None => {
            return (
                StatusCode::NOT_FOUND,
                [(header::CONTENT_TYPE, "application/json")],
                b"{\"errors\":[{\"detail\":\"no docs for this version\"}]}".to_vec(),
            )
                .into_response()
        }
    };

    let items: Vec<DocItem> = match rmp_serde::from_slice(&blob) {
        Ok(i) => i,
        Err(e) => {
            tracing::error!("docs msgpack decode failed for {name}/{resolved_version}: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "application/json")],
                b"{\"errors\":[{\"detail\":\"docs data corrupt\"}]}".to_vec(),
            )
                .into_response();
        }
    };

    let json_items: Vec<Value> = items
        .iter()
        .map(|item| {
            let tags: Vec<Value> = item
                .tags
                .iter()
                .map(|t| {
                    json!({
                        "kind":  format!("{:?}", t.kind),
                        "label": t.kind.label(),
                        "name":  t.name,
                        "text":  t.text,
                    })
                })
                .collect();

            json!({
                "name":      item.name,
                "kind":      item.kind.label(),
                "lang":      item.lang.label(),
                "brief":     item.brief,
                "body":      item.body,
                "signature": item.signature,
                "file":      item.file.to_string_lossy(),
                "line":      item.line,
                "tags":      tags,
                "meta": {
                    "template_params": item.meta.template_params,
                    "access":          item.meta.access.as_ref().map(Access::label),
                    "parent":          item.meta.parent,
                    "attrs":           item.meta.attrs,
                    "group":           item.meta.group,
                },
            })
        })
        .collect();

    let body = serde_json::to_vec(&json!({ "items": json_items, "total": json_items.len() }))
        .unwrap_or_default();

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}
