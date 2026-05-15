use crate::api::ApiError;

/// Package names that cannot be registered.
static RESERVED: &[&str] = &[
    "std", "core", "alloc", "test", "proc-macro", "build", "bench",
    "example", "examples", "src", "lib", "bin", "bins", "registry",
    "freight", "crate", "package",
];

/// Validate a package name.
///
/// Accepts two forms:
/// - Plain name:  `mylib`  — 1–64 ASCII alphanumeric / hyphen / underscore
/// - Scoped name: `@scope/mylib` — scope is 1–32 chars, name follows the same rules
///
/// Scoped names must use URL percent-encoding (`%2F`) in API paths.
pub fn package_name(name: &str) -> Result<(), ApiError> {
    if let Some(rest) = name.strip_prefix('@') {
        let (scope, base) = rest.split_once('/').ok_or_else(|| {
            ApiError::bad_request("scoped package name must be in the form @scope/name")
        })?;
        validate_scope_part(scope)?;
        validate_base_name(base)?;
        return Ok(());
    }
    validate_base_name(name)
}

/// Extract the base name from a (potentially scoped) package name.
/// `@acme/mylib` → `"mylib"`;  `mylib` → `"mylib"`.
pub fn base_name(name: &str) -> &str {
    if let Some(rest) = name.strip_prefix('@') {
        if let Some((_, base)) = rest.split_once('/') {
            return base;
        }
    }
    name
}

fn validate_scope_part(scope: &str) -> Result<(), ApiError> {
    if scope.is_empty() || scope.len() > 32 {
        return Err(ApiError::bad_request("scope must be 1–32 characters"));
    }
    if !scope.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err(ApiError::bad_request(
            "scope may only contain ASCII letters, digits, hyphens, and underscores",
        ));
    }
    if !scope.as_bytes()[0].is_ascii_alphabetic() {
        return Err(ApiError::bad_request("scope must start with a letter"));
    }
    Ok(())
}

fn validate_base_name(name: &str) -> Result<(), ApiError> {
    if name.is_empty() || name.len() > 64 {
        return Err(ApiError::bad_request("package name must be 1–64 characters"));
    }
    if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err(ApiError::bad_request(
            "package name may only contain ASCII letters, digits, hyphens, and underscores",
        ));
    }
    let first = name.as_bytes()[0];
    let last  = *name.as_bytes().last().unwrap();
    if first == b'-' || first == b'_' || last == b'-' || last == b'_' {
        return Err(ApiError::bad_request(
            "package name cannot start or end with a hyphen or underscore",
        ));
    }
    if name.contains("--") || name.contains("__") || name.contains("-_") || name.contains("_-") {
        return Err(ApiError::bad_request(
            "package name cannot contain consecutive or mixed separator characters",
        ));
    }
    if RESERVED.iter().any(|&r| r.eq_ignore_ascii_case(name)) {
        return Err(ApiError::bad_request(format!("`{name}` is a reserved name")));
    }
    Ok(())
}

/// Validate a token scope string. Accepted values: `"read"`, `"publish"`, `"admin"`.
pub fn token_scope(scope: &str) -> Result<(), ApiError> {
    match scope {
        "read" | "publish" | "admin" => Ok(()),
        _ => Err(ApiError::bad_request(
            "invalid token scope — must be \"read\", \"publish\", or \"admin\"",
        )),
    }
}

/// Validate a version string — requires at least `major.minor` semver structure.
pub fn version(vers: &str) -> Result<(), ApiError> {
    if vers.is_empty() || vers.len() > 64 {
        return Err(ApiError::bad_request("version must be 1–64 characters"));
    }
    // Strip pre-release and build metadata.
    let core = vers.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return Err(ApiError::bad_request(
            "version must be semver: major.minor[.patch]",
        ));
    }
    for part in &parts {
        if part.parse::<u64>().is_err() {
            return Err(ApiError::bad_request(format!(
                "invalid version component `{part}` — must be a non-negative integer"
            )));
        }
    }
    Ok(())
}

/// Validate a username:
/// - 2–32 ASCII alphanumeric / hyphen / underscore characters
/// - Must start with a letter
pub fn username(name: &str) -> Result<(), String> {
    if name.len() < 2 || name.len() > 32 {
        return Err("username must be 2–32 characters".into());
    }
    if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err("username may only contain letters, digits, hyphens, and underscores".into());
    }
    if !name.as_bytes()[0].is_ascii_alphabetic() {
        return Err("username must start with a letter".into());
    }
    Ok(())
}

/// Validate a password (minimum 8 characters).
pub fn password(pw: &str) -> Result<(), String> {
    if pw.len() < 8 {
        return Err("password must be at least 8 characters".into());
    }
    Ok(())
}
