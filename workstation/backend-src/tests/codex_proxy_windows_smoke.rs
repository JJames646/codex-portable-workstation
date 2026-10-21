#![cfg(windows)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use yunshu_workstation_runtime::codex_proxy::{
    CodexProxySubscriptionScanInput, mark_start_once, prepare_codex_launch, scan_subscription,
    status as proxy_status,
};
use yunshu_workstation_runtime::paths::WorkstationPaths;

#[test]
#[ignore = "requires YUNSHU_MIHOMO_TEST_EXE pointing to the verified Windows binary"]
fn subscription_scan_and_cached_launch_work_with_staged_mihomo() {
    let mihomo = std::env::var_os("YUNSHU_MIHOMO_TEST_EXE")
        .expect("YUNSHU_MIHOMO_TEST_EXE must point to the verified Windows binary");
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("portable");
    let runtime = root.join("runtimes").join("mihomo");
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::copy(mihomo, runtime.join("mihomo.exe")).unwrap();

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request);
        let body = "proxies:\n  - name: \"HK Smoke\"\n    type: socks5\n    server: 127.0.0.1\n    port: 9\n    udp: false\n";
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/yaml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });

    let paths = WorkstationPaths::from_root(root);
    let status = scan_subscription(
        &paths,
        CodexProxySubscriptionScanInput {
            subscription_url: Some(format!("http://{address}/subscription?token=smoke-secret")),
        },
    )
    .unwrap();
    server.join().unwrap();

    assert_eq!(status.node_count, 1);
    assert_eq!(status.selected_node.as_deref(), Some("HK Smoke"));
    assert_eq!(
        status.subscription_masked.as_deref(),
        Some("http://127.0.0.1/***")
    );
    assert!(!status.active);

    mark_start_once(&paths).unwrap();
    let session = prepare_codex_launch(&paths).unwrap().unwrap();
    assert!(proxy_status(&paths).unwrap().active);
    assert!(
        session
            .chromium_proxy_argument()
            .to_string_lossy()
            .starts_with("--proxy-server=http://127.0.0.1:")
    );
    drop(session);
    assert!(!proxy_status(&paths).unwrap().active);
}
