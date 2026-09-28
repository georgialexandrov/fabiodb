//! SSL modes against the dev server, which offers TLS with a private CA
//! (dev/pg.sh). Where the server has no TLS, the connecting tests say so and
//! pass without checking — unless FABIO_TEST_REQUIRE_TLS is set, as in CI.

mod common;

use std::path::PathBuf;

use common::{postgres, postgres_target};
use fabiodb_core::{Db, PgTarget, SslMode, Target};

fn ca() -> PathBuf {
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../dev/.pgdata/ssl/ca.crt").into()
}

async fn server_has_tls() -> bool {
    let on = postgres().await.query("show ssl").await.unwrap().rows[0][0].as_deref() == Some("on");
    let required = std::env::var_os("FABIO_TEST_REQUIRE_TLS").is_some();
    assert!(!required || (on && ca().exists()), "FABIO_TEST_REQUIRE_TLS: the server has no TLS or dev CA");
    if !on {
        eprintln!("server has ssl = off; TLS checks skipped");
    }
    on && ca().exists()
}

fn target(host: &str, ssl: SslMode, ca_cert: Option<PathBuf>) -> Target {
    let Target::Postgres(mut t) = postgres_target() else { unreachable!() };
    t.host = host.into();
    t.ssl = ssl;
    t.ca_cert = ca_cert;
    Target::Postgres(t)
}

async fn uses_tls(db: &Db) -> bool {
    let sql = "select ssl::text from pg_stat_ssl where pid = pg_backend_pid()";
    db.query(sql).await.unwrap().rows[0][0].as_deref() == Some("true")
}

#[tokio::test]
async fn verify_full_connects_with_the_right_ca_and_host() {
    if !server_has_tls().await {
        return;
    }
    let db = Db::open(&target("localhost", SslMode::VerifyFull, Some(ca()))).await.unwrap();
    assert!(uses_tls(&db).await);
}

#[tokio::test]
async fn verify_full_refuses_an_unknown_ca() {
    if !server_has_tls().await {
        return;
    }
    let err = Db::open(&target("localhost", SslMode::VerifyFull, None)).await.err().unwrap();
    assert!(err.to_string().to_lowercase().contains("certificate"), "{err}");
}

#[tokio::test]
async fn verify_full_refuses_the_wrong_host_but_verify_ca_accepts_it() {
    if !server_has_tls().await {
        return;
    }
    assert!(Db::open(&target("127.0.0.1", SslMode::VerifyFull, Some(ca()))).await.is_err());
    let db = Db::open(&target("127.0.0.1", SslMode::VerifyCa, Some(ca()))).await.unwrap();
    assert!(uses_tls(&db).await);
}

#[tokio::test]
async fn require_encrypts_without_checking() {
    if !server_has_tls().await {
        return;
    }
    let db = Db::open(&target("127.0.0.1", SslMode::Require, None)).await.unwrap();
    assert!(uses_tls(&db).await);
}

#[test]
fn url_carries_verify_modes_and_the_ca_file() {
    let t = PgTarget::from_url("postgres://me@db.example/app?sslmode=verify-full&sslrootcert=/etc/ca.pem").unwrap();
    assert_eq!(t.ssl, SslMode::VerifyFull);
    assert_eq!(t.ca_cert, Some(PathBuf::from("/etc/ca.pem")));

    let t = PgTarget::from_url("postgresql://me@db.example:6543/app?application_name=x&sslmode=verify-ca").unwrap();
    assert_eq!((t.ssl, t.port), (SslMode::VerifyCa, 6543));
}

/// A bundle holds many CAs, and the one that signed the server can be anywhere
/// in it. Amazon's RDS bundle starts with sa-east-1; reading only the first
/// certificate is how Fabio failed its first RDS connection.
#[tokio::test]
async fn verify_full_trusts_every_ca_in_a_bundle() {
    if !server_has_tls().await {
        return;
    }
    let rds = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/certs/rds-global-bundle.pem")).unwrap();
    let mut bundle = rds;
    bundle.extend(std::fs::read(ca()).unwrap());
    let path = std::env::temp_dir().join(format!("fabio-bundle-{}.pem", std::process::id()));
    std::fs::write(&path, bundle).unwrap();

    let db = Db::open(&target("localhost", SslMode::VerifyFull, Some(path.clone()))).await.unwrap();
    assert!(uses_tls(&db).await);
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn a_ca_file_without_certificates_is_named() {
    let path = std::env::temp_dir().join(format!("fabio-not-pem-{}.pem", std::process::id()));
    std::fs::write(&path, "not a certificate").unwrap();
    let err = Db::open(&target("localhost", SslMode::VerifyFull, Some(path.clone()))).await.err().unwrap();
    assert!(err.to_string().contains(&path.display().to_string()), "{err}");
    std::fs::remove_file(path).unwrap();
}
