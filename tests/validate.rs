use freight_registry::validate;

// ── package_name ──────────────────────────────────────────────────────────────

#[test]
fn package_name_valid() {
    assert!(validate::package_name("my-lib").is_ok());
    assert!(validate::package_name("mylib").is_ok());
    assert!(validate::package_name("my_lib").is_ok());
    assert!(validate::package_name("mylib123").is_ok());
    assert!(validate::package_name("a").is_ok());
    assert!(validate::package_name(&"a".repeat(64)).is_ok());
}

#[test]
fn package_name_empty() {
    assert!(validate::package_name("").is_err());
}

#[test]
fn package_name_too_long() {
    assert!(validate::package_name(&"a".repeat(65)).is_err());
}

#[test]
fn package_name_invalid_chars() {
    assert!(validate::package_name("my lib").is_err());
    assert!(validate::package_name("my.lib").is_err());
    assert!(validate::package_name("my/lib").is_err());
    assert!(validate::package_name("my@lib").is_err());
}

#[test]
fn package_name_leading_trailing_separator() {
    assert!(validate::package_name("-mylib").is_err());
    assert!(validate::package_name("mylib-").is_err());
    assert!(validate::package_name("_mylib").is_err());
    assert!(validate::package_name("mylib_").is_err());
}

#[test]
fn package_name_consecutive_separators() {
    assert!(validate::package_name("my--lib").is_err());
    assert!(validate::package_name("my__lib").is_err());
    assert!(validate::package_name("my-_lib").is_err());
    assert!(validate::package_name("my_-lib").is_err());
}

#[test]
fn package_name_reserved() {
    assert!(validate::package_name("std").is_err());
    assert!(validate::package_name("core").is_err());
    assert!(validate::package_name("freight").is_err());
    assert!(validate::package_name("registry").is_err());
    assert!(validate::package_name("STD").is_err()); // case-insensitive
}

// ── version ───────────────────────────────────────────────────────────────────

#[test]
fn version_valid() {
    assert!(validate::version("1.0.0").is_ok());
    assert!(validate::version("1.0").is_ok());
    assert!(validate::version("0.1.0").is_ok());
    assert!(validate::version("1.0.0-alpha").is_ok());
    assert!(validate::version("1.0.0+build.1").is_ok());
    assert!(validate::version("1.0.0-alpha.1+build").is_ok());
}

#[test]
fn version_empty() {
    assert!(validate::version("").is_err());
}

#[test]
fn version_single_component() {
    assert!(validate::version("1").is_err());
}

#[test]
fn version_too_many_components() {
    assert!(validate::version("1.2.3.4").is_err());
}

#[test]
fn version_non_numeric() {
    assert!(validate::version("a.b.c").is_err());
    assert!(validate::version("1.x.0").is_err());
}

// ── username ──────────────────────────────────────────────────────────────────

#[test]
fn username_valid() {
    assert!(validate::username("alice").is_ok());
    assert!(validate::username("al").is_ok()); // min length 2
    assert!(validate::username("alice123").is_ok());
    assert!(validate::username("alice-bob").is_ok());
    assert!(validate::username("alice_bob").is_ok());
    assert!(validate::username(&"a".repeat(32)).is_ok()); // max length 32
}

#[test]
fn username_too_short() {
    assert!(validate::username("a").is_err());
    assert!(validate::username("").is_err());
}

#[test]
fn username_too_long() {
    assert!(validate::username(&"a".repeat(33)).is_err());
}

#[test]
fn username_starts_with_digit() {
    assert!(validate::username("1alice").is_err());
    assert!(validate::username("123").is_err());
}

#[test]
fn username_invalid_chars() {
    assert!(validate::username("ali ce").is_err());
    assert!(validate::username("ali@ce").is_err());
    assert!(validate::username("ali.ce").is_err());
}

// ── password ──────────────────────────────────────────────────────────────────

#[test]
fn password_valid() {
    assert!(validate::password("12345678").is_ok());
    assert!(validate::password("a very long password!!").is_ok());
}

#[test]
fn password_too_short() {
    assert!(validate::password("1234567").is_err()); // 7 chars
    assert!(validate::password("").is_err());
}
