//! Tests of the token store (trait + implementations).

use tg_auth::{KeyringStore, MemoryStore, TokenStore};

#[test]
fn memory_store_roundtrip() {
    let store = MemoryStore::default();
    assert_eq!(store.get("github.com").unwrap(), None);

    store.set("github.com", "tok-1").unwrap();
    assert_eq!(store.get("github.com").unwrap(), Some("tok-1".into()));

    // Overwriting replaces the value.
    store.set("github.com", "tok-2").unwrap();
    assert_eq!(store.get("github.com").unwrap(), Some("tok-2".into()));

    // Other hosts stay separate.
    store.set("gitlab.example.com", "tok-gl").unwrap();
    assert_eq!(store.get("github.com").unwrap(), Some("tok-2".into()));

    store.delete("github.com").unwrap();
    assert_eq!(store.get("github.com").unwrap(), None);
    // Deleting a missing entry is not an error (idempotent).
    store.delete("github.com").unwrap();
}

/// A real OS keychain roundtrip. Ignored by default because CI containers have
/// no secret service — locally: `cargo test -p tg-auth -- --ignored`.
#[test]
#[ignore = "needs the OS keychain (run locally with --ignored)"]
fn keyring_store_roundtrip() {
    let store = KeyringStore::with_service("terra-git-test");
    let host = "test.terra-git.invalid";
    store.delete(host).unwrap(); // leftovers from earlier runs

    assert_eq!(store.get(host).unwrap(), None);
    store.set(host, "secret-123").unwrap();
    assert_eq!(store.get(host).unwrap(), Some("secret-123".into()));
    store.set(host, "secret-456").unwrap();
    assert_eq!(store.get(host).unwrap(), Some("secret-456".into()));

    store.delete(host).unwrap();
    assert_eq!(store.get(host).unwrap(), None);
}
