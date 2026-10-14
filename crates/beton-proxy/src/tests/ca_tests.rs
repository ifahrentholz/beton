//! Tests der beton-CA (PRX-002).

use std::path::Path;
use std::sync::Arc;

use beton_secrets::backend::{Backend, FileBackend, KdfParams, MemoryBackend};
use beton_secrets::{SecretValue, Secrets, UseCapabilities, UseContext};
use beton_store::{AuditQuery, Store, StoreOptions};
use rustls::client::danger::ServerCertVerifier;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use time::OffsetDateTime;

use super::*;

/// Prüft eine Kette wie ein Client, der nur der beton-CA vertraut.
fn verify(ca: &BetonCa, leaf: &CertifiedKey, name: &str) -> Result<(), rustls::Error> {
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from(ca.cert_der().to_vec()))
        .unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier =
        rustls::client::WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider)
            .build()
            .unwrap();
    verifier
        .verify_server_cert(
            &leaf.cert[0],
            &leaf.cert[1..],
            &ServerName::try_from(name.to_owned()).unwrap(),
            &[],
            UnixTime::now(),
        )
        .map(|_| ())
}

fn host(s: &str) -> Host {
    Host::parse(s).unwrap()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn prx_002_ca_is_p256_with_two_years_and_installation_cn() {
    let now = OffsetDateTime::now_utc();
    let ca = BetonCa::generate("nod_01TEST", now).unwrap();
    let info = ca.info();
    assert_eq!(info.subject, "beton egress CA nod_01TEST");
    assert!(contains(ca.cert_der(), b"beton egress CA nod_01TEST"));
    let days =
        (info.not_after.as_offset_date_time() - info.not_before.as_offset_date_time()).whole_days();
    assert!((729..=731).contains(&days), "{days} Tage");
    assert_eq!(info.fingerprint_sha256.len(), 64);
    assert!(ca.cert_pem().starts_with("-----BEGIN CERTIFICATE-----"));
    // ECDSA P-256: OID prime256v1 im SPKI.
    assert!(contains(
        ca.cert_der(),
        &[0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07]
    ));
    let other = BetonCa::generate("nod_01TEST", now).unwrap();
    assert_ne!(other.info().fingerprint_sha256, info.fingerprint_sha256);
}

#[test]
fn prx_002_leafs_chain_to_the_ca_with_host_san() {
    let ca = Arc::new(BetonCa::generate("nod_x", OffsetDateTime::now_utc()).unwrap());
    let issuer = LeafIssuer::new(ca.clone()).unwrap();
    let now = OffsetDateTime::now_utc();
    let leaf = issuer.certified_key(&host("api.github.com"), now).unwrap();
    verify(&ca, &leaf, "api.github.com").unwrap();
    assert!(verify(&ca, &leaf, "evil.test").is_err(), "SAN = Host");
    let ip = issuer.certified_key(&host("93.184.216.34"), now).unwrap();
    verify(&ca, &ip, "93.184.216.34").unwrap();
    // Einer fremden CA vertraut der Client nicht.
    let foreign = BetonCa::generate("nod_y", now).unwrap();
    assert!(verify(&foreign, &leaf, "api.github.com").is_err());
}

#[test]
fn prx_002_leafs_share_one_key_per_instance_and_are_cached() {
    let ca = Arc::new(BetonCa::generate("nod_x", OffsetDateTime::now_utc()).unwrap());
    let now = OffsetDateTime::now_utc();
    let issuer = LeafIssuer::new(ca.clone()).unwrap();
    let a = issuer.certified_key(&host("a.test"), now).unwrap();
    let b = issuer.certified_key(&host("b.test"), now).unwrap();
    let spki_a = a.key.public_key().unwrap();
    let spki_b = b.key.public_key().unwrap();
    assert_eq!(
        spki_a.as_ref(),
        spki_b.as_ref(),
        "ein Schlüssel pro Instanz"
    );
    assert!(contains(a.cert[0].as_ref(), spki_a.as_ref()));
    assert!(contains(b.cert[0].as_ref(), spki_a.as_ref()));
    assert_ne!(a.cert[0].as_ref(), b.cert[0].as_ref());
    // Cache: derselbe Host liefert dasselbe Zertifikat.
    let again = issuer.certified_key(&host("a.test"), now).unwrap();
    assert!(Arc::ptr_eq(&a, &again));
    // Nach 23 h ist es fast abgelaufen und wird erneuert.
    let later = issuer
        .certified_key(
            &host("a.test"),
            now + time::Duration::hours(23) + time::Duration::minutes(30),
        )
        .unwrap();
    assert!(!Arc::ptr_eq(&a, &later));
    // Eine andere Instanz hat einen anderen Schlüssel.
    let other = LeafIssuer::new(ca).unwrap();
    let c = other.certified_key(&host("a.test"), now).unwrap();
    assert_ne!(c.key.public_key().unwrap().as_ref(), spki_a.as_ref());
}

struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
    secrets: Secrets,
    caps: UseCapabilities,
    cas: CaStore,
}

async fn fixture_with(dir: tempfile::TempDir, backend: Arc<dyn Backend>) -> Fixture {
    let store = Store::open(dir.path(), StoreOptions::default())
        .await
        .unwrap();
    let local = store.ensure_local().await.unwrap();
    let (secrets, caps) = Secrets::new(store.clone(), local, backend);
    let cas = CaStore::new(secrets.clone(), store.clone(), local);
    Fixture {
        dir,
        store,
        secrets,
        caps,
        cas,
    }
}

fn ctx() -> UseContext {
    UseContext {
        session_id: None,
        host_id: "hst_local".into(),
    }
}

async fn actions(store: &Store) -> Vec<(String, serde_json::Value)> {
    let local = store.ensure_local().await.unwrap();
    store
        .audit_page(
            local.org,
            &AuditQuery {
                limit: 100,
                ..AuditQuery::default()
            },
        )
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|e| (e.action, e.details))
        .collect()
}

fn cheap_kdf() -> KdfParams {
    KdfParams {
        m_kib: 64,
        t: 1,
        p: 1,
    }
}

#[tokio::test]
async fn prx_002_ca_is_created_once_and_loaded_via_proxy_capability() {
    let f = fixture_with(
        tempfile::tempdir().unwrap(),
        Arc::new(MemoryBackend::default()),
    )
    .await;
    let first = f.cas.load_or_create(&f.caps.proxy, ctx()).await.unwrap();
    let second = f.cas.load_or_create(&f.caps.proxy, ctx()).await.unwrap();
    assert_eq!(first.info(), second.info());
    let local = f.store.ensure_local().await.unwrap();
    assert!(first.info().subject.ends_with(&local.node.to_string()));
    let meta = f.secrets.list_meta().await.unwrap();
    assert_eq!(meta.len(), 1);
    assert_eq!(meta[0].secret_ref, CA_SECRET_REF);
    let created = actions(&f.store)
        .await
        .into_iter()
        .filter(|(a, _)| a == "secret.created")
        .count();
    assert_eq!(created, 1);
    // Die Nutzung ist auditiert (SEC-001 AC3); bei Proxy-Zweck als Nutzungsfenster.
    assert!(
        f.secrets.list_meta().await.unwrap()[0]
            .last_used_at
            .is_some()
    );
}

#[tokio::test]
async fn prx_002_ac3_ca_key_is_not_in_plaintext_under_beton_home() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(FileBackend::new(
        FileBackend::default_path(dir.path()),
        cheap_kdf(),
    ));
    let f = fixture_with(dir, backend).await;
    f.secrets
        .unlock(SecretValue::from("test-passphrase"))
        .await
        .unwrap();
    let ca = f.cas.load_or_create(&f.caps.proxy, ctx()).await.unwrap();
    let key = ca.key_pkcs8_der();
    assert!(key.len() > 32);
    let key_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &key);
    assert_no_key_material(f.dir.path(), &key, key_b64.as_bytes());
}

/// Durchsucht ein Verzeichnis nach PEM-Key-Markern und dem Schlüssel selbst.
pub(crate) fn assert_no_key_material(dir: &Path, key: &[u8], key_b64: &[u8]) {
    let mut stack = vec![dir.to_path_buf()];
    let mut files = 0;
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            files += 1;
            let bytes = std::fs::read(&path).unwrap();
            for marker in [&b"PRIVATE KEY"[..], key, &key_b64[..40]] {
                assert!(
                    !contains(&bytes, marker),
                    "Schlüsselmaterial im Klartext in {}",
                    path.display()
                );
            }
        }
    }
    assert!(files > 0, "nichts durchsucht");
}

#[tokio::test]
async fn prx_002_locked_store_yields_no_ca() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(FileBackend::new(
        FileBackend::default_path(dir.path()),
        cheap_kdf(),
    ));
    let f = fixture_with(dir, backend).await;
    let err = f
        .cas
        .load_or_create(&f.caps.proxy, ctx())
        .await
        .unwrap_err();
    assert_eq!(err.code(), "secrets_locked");
    assert!(f.secrets.list_meta().await.unwrap().is_empty());
    let err = f.cas.rotate().await.unwrap_err();
    assert_eq!(err.code(), "secrets_locked");
}

#[tokio::test]
async fn prx_002_ac4_rotation_creates_a_new_ca_and_is_audited() {
    let f = fixture_with(
        tempfile::tempdir().unwrap(),
        Arc::new(MemoryBackend::default()),
    )
    .await;
    let old = f.cas.load_or_create(&f.caps.proxy, ctx()).await.unwrap();
    let info = f.cas.rotate().await.unwrap();
    assert_ne!(info.fingerprint_sha256, old.info().fingerprint_sha256);
    let new = f.cas.load_or_create(&f.caps.proxy, ctx()).await.unwrap();
    assert_eq!(new.info(), info, "neue Sessions nutzen die neue CA");
    let audit = actions(&f.store).await;
    let rotated: Vec<_> = audit
        .iter()
        .filter(|(a, _)| a == "proxy.ca_rotated")
        .collect();
    assert_eq!(rotated.len(), 1, "{audit:?}");
    assert_eq!(
        rotated[0].1["fingerprint_sha256"],
        serde_json::json!(info.fingerprint_sha256)
    );
    assert_eq!(
        rotated[0].1["previous_fingerprint_sha256"],
        serde_json::json!(old.info().fingerprint_sha256)
    );
    // Laufende Proxys behalten ihre CA: Leafs der alten CA bleiben gültig gegen die alte.
    let old = Arc::new(old);
    let leaf = LeafIssuer::new(old.clone())
        .unwrap()
        .certified_key(&host("a.test"), OffsetDateTime::now_utc())
        .unwrap();
    verify(&old, &leaf, "a.test").unwrap();
    assert!(verify(&new, &leaf, "a.test").is_err());
}

#[tokio::test]
async fn prx_002_rotation_without_existing_ca_creates_one() {
    let f = fixture_with(
        tempfile::tempdir().unwrap(),
        Arc::new(MemoryBackend::default()),
    )
    .await;
    let info = f.cas.rotate().await.unwrap();
    let loaded = f.cas.load_or_create(&f.caps.proxy, ctx()).await.unwrap();
    assert_eq!(loaded.info(), info);
    assert!(
        actions(&f.store)
            .await
            .iter()
            .any(|(a, _)| a == "proxy.ca_rotated")
    );
}

#[tokio::test]
async fn prx_002_corrupt_or_expired_ca_fails_closed() {
    let f = fixture_with(
        tempfile::tempdir().unwrap(),
        Arc::new(MemoryBackend::default()),
    )
    .await;
    let r: beton_secrets::SecretRef = CA_SECRET_REF.parse().unwrap();
    f.secrets
        .put(beton_secrets::NewSecret {
            secret_ref: r.clone(),
            kind: beton_secrets::SecretKind::Token,
            value: SecretValue::from("kein json"),
        })
        .await
        .unwrap();
    let err = f
        .cas
        .load_or_create(&f.caps.proxy, ctx())
        .await
        .unwrap_err();
    assert_eq!(err.code(), "proxy_ca_corrupt");

    let expired = BetonCa::generate(
        "nod_x",
        OffsetDateTime::now_utc() - time::Duration::days(800),
    )
    .unwrap();
    f.secrets
        .rotate(&r, expired.to_secret().unwrap())
        .await
        .unwrap();
    let err = f
        .cas
        .load_or_create(&f.caps.proxy, ctx())
        .await
        .unwrap_err();
    assert_eq!(err.code(), "proxy_ca_expired");
}

#[test]
fn prx_002_ac1_ca_is_never_in_a_system_trust_store() {
    let ca = BetonCa::generate("nod_x", OffsetDateTime::now_utc()).unwrap();
    let native = rustls_native_certs::load_native_certs();
    assert!(
        native.certs.iter().all(|c| c.as_ref() != ca.cert_der()),
        "beton-CA im System-Trust-Store"
    );
    // Der Proxy-Code installiert nie eine CA in einen Trust-Store.
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![src];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "tests") {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for needle in [
                "add-trusted-cert",
                "update-ca-certificates",
                "update-ca-trust",
                "certutil",
                "trust anchor",
                "/usr/local/share/ca-certificates",
                "/etc/pki/ca-trust",
                "Command::new",
            ] {
                assert!(
                    !text.contains(needle),
                    "{} enthält `{needle}`",
                    path.display()
                );
            }
        }
    }
}
