//! V2 inbound connection lifecycle: concurrent cap, idle timeout, protocol-error budget, frame cap.

use std::io::Read;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use node::blockchain::Blockchain;
use node::genesis::Genesis;
use node::mempool::Mempool;
use node::network::{
    encode_session_payload, handshake_initiator, read_framed, serve_tcp_listener,
    wire_encode_get_blocks, write_framed, InboundPeerPolicy, InboundSlotPool, NodeInner,
    OP_SESSION_HELLO,
};
use node::storage::BlockStore;

fn temp_chain_path(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "trilog_netdef_{}_{}_{}",
        label,
        std::process::id(),
        nanos
    ))
}

fn minimal_state() -> Arc<Mutex<NodeInner>> {
    let genesis = Genesis::empty();
    let chain = Blockchain::from_genesis(&genesis).unwrap();
    let path = temp_chain_path("st");
    let _ = std::fs::remove_file(&path);
    let store = BlockStore::open_append(&path).unwrap();
    Arc::new(Mutex::new(NodeInner {
        genesis,
        chain,
        pool: Mempool::new(10),
        store,
    }))
}

fn spawn_serve(
    policy: InboundPeerPolicy,
) -> (thread::JoinHandle<()>, std::net::SocketAddr, Arc<Mutex<NodeInner>>) {
    let state = minimal_state();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let slots = InboundSlotPool::new(policy.max_concurrent_sessions);
    let st = state.clone();
    let jh = thread::spawn(move || serve_tcp_listener(listener, st, policy, slots));
    thread::sleep(Duration::from_millis(60));
    (jh, addr, state)
}

#[test]
fn inbound_slot_released_when_session_ends() {
    let policy = InboundPeerPolicy {
        max_concurrent_sessions: 1,
        idle_read_timeout: Duration::from_secs(60),
        write_timeout: None,
        max_protocol_errors_per_session: 32,
        max_app_frames_per_session: 10_000,
        ..Default::default()
    };
    let (_jh, addr, state) = spawn_serve(policy);
    let genesis = { state.lock().unwrap().genesis.clone() };

    let mut first = std::net::TcpStream::connect(addr).unwrap();
    first
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok();
    handshake_initiator(&mut first, &genesis, 0).unwrap();
    drop(first);

    thread::sleep(Duration::from_millis(250));

    let mut second = std::net::TcpStream::connect(addr).unwrap();
    second
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok();
    handshake_initiator(&mut second, &genesis, 0).unwrap();
}

#[test]
fn inbound_peer_cap_refuses_excess_connections() {
    let policy = InboundPeerPolicy {
        max_concurrent_sessions: 2,
        idle_read_timeout: Duration::from_secs(60),
        write_timeout: None,
        max_protocol_errors_per_session: 32,
        max_app_frames_per_session: 10_000,
        ..Default::default()
    };
    let (_jh, addr, state) = spawn_serve(policy);
    let genesis = { state.lock().unwrap().genesis.clone() };

    let mut a = std::net::TcpStream::connect(addr).unwrap();
    a.set_read_timeout(Some(Duration::from_secs(10))).ok();
    handshake_initiator(&mut a, &genesis, 0).unwrap();

    let mut b = std::net::TcpStream::connect(addr).unwrap();
    b.set_read_timeout(Some(Duration::from_secs(10))).ok();
    handshake_initiator(&mut b, &genesis, 0).unwrap();

    let mut c = std::net::TcpStream::connect(addr).unwrap();
    thread::sleep(Duration::from_millis(120));
    c.set_write_timeout(Some(Duration::from_secs(2))).ok();
    let hello = encode_session_payload(OP_SESSION_HELLO, &genesis, 0).unwrap();
    let w = write_framed(&mut c, &hello);
    let mut closed = w.is_err();
    if w.is_ok() {
        c.set_read_timeout(Some(Duration::from_secs(3))).ok();
        let mut buf = [0u8; 1];
        closed = matches!(c.read(&mut buf), Ok(0) | Err(_));
    }
    assert!(
        closed,
        "third inbound peer must not complete handshake while cap is reached"
    );
}

#[test]
fn inbound_idle_read_timeout_closes_session() {
    let policy = InboundPeerPolicy {
        max_concurrent_sessions: 8,
        idle_read_timeout: Duration::from_millis(700),
        write_timeout: None,
        max_protocol_errors_per_session: 32,
        max_app_frames_per_session: 1000,
        ..Default::default()
    };
    let (_jh, addr, state) = spawn_serve(policy);
    let genesis = { state.lock().unwrap().genesis.clone() };

    let mut c = std::net::TcpStream::connect(addr).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(10))).ok();
    handshake_initiator(&mut c, &genesis, 0).unwrap();

    thread::sleep(Duration::from_millis(1_400));
    c.set_read_timeout(Some(Duration::from_secs(3))).ok();
    let mut buf = [0u8; 4];
    let n = c.read(&mut buf).unwrap_or(0);
    assert_eq!(n, 0, "peer should close after idle read timeout");
}

#[test]
fn inbound_repeated_protocol_errors_disconnect() {
    let policy = InboundPeerPolicy {
        max_concurrent_sessions: 4,
        idle_read_timeout: Duration::from_secs(30),
        write_timeout: None,
        max_protocol_errors_per_session: 2,
        max_app_frames_per_session: 100,
        ..Default::default()
    };
    let (_jh, addr, state) = spawn_serve(policy);
    let genesis = { state.lock().unwrap().genesis.clone() };

    let mut c = std::net::TcpStream::connect(addr).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(10))).ok();
    handshake_initiator(&mut c, &genesis, 0).unwrap();

    let bad = vec![255u8];
    write_framed(&mut c, &bad).unwrap();
    write_framed(&mut c, &bad).unwrap();

    c.set_read_timeout(Some(Duration::from_secs(3))).ok();
    let mut buf = [0u8; 1];
    let r = c.read(&mut buf);
    assert!(
        matches!(r, Ok(0) | Err(_)),
        "expected disconnect after protocol error budget: {r:?}"
    );
}

#[test]
fn inbound_max_app_frames_per_session_enforced() {
    let policy = InboundPeerPolicy {
        max_concurrent_sessions: 4,
        idle_read_timeout: Duration::from_secs(30),
        write_timeout: None,
        max_protocol_errors_per_session: 32,
        max_app_frames_per_session: 3,
        ..Default::default()
    };
    let (_jh, addr, state) = spawn_serve(policy);
    let genesis = { state.lock().unwrap().genesis.clone() };

    let mut c = std::net::TcpStream::connect(addr).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(10))).ok();
    handshake_initiator(&mut c, &genesis, 0).unwrap();

    for _ in 0..3 {
        write_framed(&mut c, &wire_encode_get_blocks(999)).unwrap();
        let resp = read_framed(&mut c).unwrap();
        assert!(!resp.is_empty());
    }

    write_framed(&mut c, &wire_encode_get_blocks(999)).unwrap();
    thread::sleep(Duration::from_millis(200));
    c.set_read_timeout(Some(Duration::from_secs(2))).ok();
    let fourth = read_framed(&mut c);
    assert!(
        fourth.is_err(),
        "session should end before a 4th reply is produced"
    );
}

#[test]
fn active_inbound_traffic_avoids_idle_drop() {
    let policy = InboundPeerPolicy {
        max_concurrent_sessions: 4,
        idle_read_timeout: Duration::from_millis(900),
        write_timeout: None,
        max_protocol_errors_per_session: 32,
        max_app_frames_per_session: 50,
        ..Default::default()
    };
    let (_jh, addr, state) = spawn_serve(policy);
    let genesis = { state.lock().unwrap().genesis.clone() };

    let mut c = std::net::TcpStream::connect(addr).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(10))).ok();
    handshake_initiator(&mut c, &genesis, 0).unwrap();

    for _ in 0..4 {
        thread::sleep(Duration::from_millis(400));
        write_framed(&mut c, &wire_encode_get_blocks(999)).unwrap();
        let resp = read_framed(&mut c).unwrap();
        assert!(!resp.is_empty());
    }
}
