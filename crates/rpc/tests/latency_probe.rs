#![cfg(feature = "uniffi")]
//! Temporary probe: small vs 64 KB unary RPC through a relay with an emulated delay.

use std::sync::Arc;
use std::time::{Duration, Instant};

use slim_bindings::{
    Direction, IdentityProviderConfig, IdentityVerifierConfig, Name, create_service,
    initialize_with_defaults, new_insecure_client_config, new_insecure_server_config,
};
use slim_rpc::{Channel, RpcError, Server, UnaryUnaryHandler};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

struct Echo;
#[async_trait::async_trait]
impl UnaryUnaryHandler for Echo {
    async fn handle(&self, r: Vec<u8>, _c: Arc<slim_rpc::Context>) -> Result<Vec<u8>, RpcError> {
        Ok(r)
    }
}

async fn pump(
    mut r: tokio::net::tcp::OwnedReadHalf,
    mut w: tokio::net::tcp::OwnedWriteHalf,
    d: Duration,
) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(Instant, Vec<u8>)>();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 65536];
        while let Ok(n) = r.read(&mut buf).await {
            if n == 0 {
                break;
            }
            let _ = tx.send((Instant::now() + d, buf[..n].to_vec()));
        }
    });
    while let Some((at, data)) = rx.recv().await {
        tokio::time::sleep_until(at.into()).await;
        if w.write_all(&data).await.is_err() {
            break;
        }
    }
}

/// TCP proxy adding `one_way` delay in each direction.
async fn delay_proxy(listen: u16, target: u16, one_way: Duration) {
    let l = TcpListener::bind(("127.0.0.1", listen)).await.unwrap();
    tokio::spawn(async move {
        loop {
            let (c, _) = l.accept().await.unwrap();
            let s = TcpStream::connect(("127.0.0.1", target)).await.unwrap();
            c.set_nodelay(true).ok();
            s.set_nodelay(true).ok();
            let (cr, cw) = c.into_split();
            let (sr, sw) = s.into_split();
            tokio::spawn(pump(cr, sw, one_way));
            tokio::spawn(pump(sr, cw, one_way));
        }
    });
}

fn auth(id: &str) -> (IdentityProviderConfig, IdentityVerifierConfig) {
    let s = "test-secret-with-sufficient-length-for-hmac-key".to_string();
    (
        IdentityProviderConfig::SharedSecret {
            id: id.into(),
            data: s.clone(),
        },
        IdentityVerifierConfig::SharedSecret {
            id: id.into(),
            data: s,
        },
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn probe_delayed_path() {
    initialize_with_defaults();
    let delay_ms: u64 = std::env::var("PROBE_ONE_WAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(35);
    let (relay_port, proxy_port) = (46901u16, 46902u16);

    let relay = create_service("relay".into()).unwrap();
    let mut sc = new_insecure_server_config(format!("127.0.0.1:{relay_port}"));
    sc.require_header_mac = Some(false);
    relay.run_server_async(sc).await.unwrap();
    delay_proxy(proxy_port, relay_port, Duration::from_millis(delay_ms)).await;

    let name = |n: &str| Arc::new(Name::new("org".into(), "test".into(), n.into()));

    // Server side connects directly to the relay; client side goes through the delay.
    let svc_s = create_service("svc-s".into()).unwrap();
    let (p, v) = auth("s");
    let sapp = svc_s
        .create_app_with_direction_async(name("probe"), p, v, Direction::Bidirectional)
        .await
        .unwrap();
    let sconn = svc_s
        .connect_async(new_insecure_client_config(format!(
            "http://127.0.0.1:{relay_port}"
        )))
        .await
        .unwrap();
    let server = Arc::new(Server::new(&sapp, sapp.name().clone()));
    server.register_unary_unary("T".into(), "Echo".into(), Arc::new(Echo));
    sapp.subscribe_async(name("probe"), Some(sconn))
        .await
        .unwrap();
    let srv = server.clone();
    tokio::spawn(async move {
        let _ = srv.serve().await;
    });
    tokio::time::sleep(Duration::from_millis(300)).await;

    let svc_c = create_service("svc-c".into()).unwrap();
    let (p, v) = auth("c");
    let capp = svc_c
        .create_app_with_direction_async(name("probe-client"), p, v, Direction::Bidirectional)
        .await
        .unwrap();
    let cconn = svc_c
        .connect_async(new_insecure_client_config(format!(
            "http://127.0.0.1:{proxy_port}"
        )))
        .await
        .unwrap();
    capp.subscribe_async(capp.name().clone(), Some(cconn))
        .await
        .unwrap();
    capp.set_route_async(name("probe"), cconn).await.unwrap();
    let channel = Channel::new(capp, name("probe"));

    for (label, size) in [
        ("small", 16usize),
        ("64KB", 65536),
        ("small", 16),
        ("64KB", 65536),
        ("small", 16),
        ("64KB", 65536),
        ("1MB", 1 << 20),
    ] {
        let t = Instant::now();
        let r = channel
            .call_unary_async(
                "T".into(),
                "Echo".into(),
                vec![7u8; size],
                Some(Duration::from_secs(20)),
                None,
            )
            .await
            .expect("call failed");
        eprintln!(
            "PROBE {label} ({size} B): {:?} (resp {} B) [one-way delay {delay_ms} ms]",
            t.elapsed(),
            r.len()
        );
    }
}
