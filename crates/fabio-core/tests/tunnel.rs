//! SSH tunnels, with a stand-in for `ssh` (tests/fixtures/fake-ssh.py) that
//! forwards the port directly. One test binary, one FABIO_SSH, run in order.

mod common;

use std::path::PathBuf;

use common::postgres_target;
use fabio_core::{Db, SshTunnel, Target};

fn setup() -> PathBuf {
    let log = std::env::temp_dir().join(format!("fabio-fake-ssh-{}.log", std::process::id()));
    let _ = std::fs::remove_file(&log);
    // SAFETY: set before any tunnel starts; this binary's tests share one value.
    unsafe {
        std::env::set_var("FABIO_SSH", concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fake-ssh.py"));
        std::env::set_var("FAKE_SSH_LOG", &log);
    }
    log
}

fn through(ssh_host: &str) -> Target {
    let Target::Postgres(mut t) = postgres_target() else { unreachable!() };
    t.ssh = Some(SshTunnel {
        host: ssh_host.into(),
        port: Some(2222),
        user: Some("deploy".into()),
        identity_file: Some("/keys/id_ed25519".into()),
    });
    Target::Postgres(t)
}

#[tokio::test]
async fn tunnels_share_one_ssh_and_carry_the_right_arguments() {
    let log = setup();

    // Two connections through the same bastion share one ssh process.
    let a = Db::open(&through("bastion.example")).await.unwrap();
    let b = Db::open(&through("bastion.example")).await.unwrap();
    assert_eq!(a.query("select 1").await.unwrap().rows, [[Some("1".to_string())]]);
    assert_eq!(b.query("select 2").await.unwrap().rows, [[Some("2".to_string())]]);

    let calls = std::fs::read_to_string(&log).unwrap();
    let lines: Vec<_> = calls.lines().collect();
    assert_eq!(lines.len(), 1, "{calls}");
    let args = lines[0];
    for expected in
        ["-N", "BatchMode=yes", "ExitOnForwardFailure=yes", "-p 2222", "-i /keys/id_ed25519", "deploy@bastion.example"]
    {
        assert!(args.contains(expected), "missing {expected:?} in {args}");
    }
    assert!(args.contains(":localhost:54329"), "{args}");

    // A refused key says why, in ssh's words.
    let err = Db::open(&through("fail.example")).await.err().unwrap();
    assert!(err.to_string().contains("Permission denied (publickey)"), "{err}");
}
