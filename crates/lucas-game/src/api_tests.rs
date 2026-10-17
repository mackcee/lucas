use super::*;
use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair, KeyUsagePurpose};
use rustls::{ServerConfig, ServerConnection, StreamOwned, pki_types::PrivatePkcs8KeyDer};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
};

type Routes = BTreeMap<&'static str, (u16, String)>;

fn fixtures() -> Routes {
    [
        (
            "activeplayername",
            include_str!("../tests/fixtures/activeplayername.json"),
        ),
        (
            "playerlist",
            include_str!("../tests/fixtures/playerlist.json"),
        ),
        (
            "eventdata",
            include_str!("../tests/fixtures/eventdata.json"),
        ),
        (
            "gamestats",
            include_str!("../tests/fixtures/gamestats.json"),
        ),
    ]
    .into_iter()
    .map(|(path, text)| (path, (200, text.to_owned())))
    .collect()
}

/// Real HTTPS mock: tests exercise the actual reqwest/rustls transport, not just
/// an injected fetch function. No bypass verifier or plain HTTP test path.
struct MockServer {
    url: String,
    root: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MockServer {
    fn start(routes: Routes, expired: bool) -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
        let key = KeyPair::generate().unwrap();
        let root = params.self_signed(&key).unwrap().pem();
        let issuer = Issuer::new(params, key);
        // Deliberate hostname mismatch mirrors Riot's loopback certificate.
        let mut leaf_params = CertificateParams::new(vec!["riot-game.test".into()]).unwrap();
        if expired {
            leaf_params.not_before = rcgen::date_time_ymd(2000, 1, 1);
            leaf_params.not_after = rcgen::date_time_ymd(2001, 1, 1);
        }
        let leaf_key = KeyPair::generate().unwrap();
        let leaf = leaf_params.signed_by(&leaf_key, &issuer).unwrap();
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![leaf.der().clone()],
                PrivatePkcs8KeyDer::from(leaf_key.serialize_der()).into(),
            )
            .unwrap();
        let config = Arc::new(config);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("https://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread_requests = requests.clone();
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                let (socket, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    Err(e) => panic!("mock accept: {e}"),
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut tls =
                    StreamOwned::new(ServerConnection::new(config.clone()).unwrap(), socket);
                let mut request = String::new();
                let mut reader = BufReader::new(&mut tls);
                // Invalid/untrusted TLS is expected in negative tests.
                if reader.read_line(&mut request).is_err() {
                    continue;
                }
                let path = request.split_whitespace().nth(1).unwrap_or("").to_owned();
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) if line == "\r\n" => break,
                        _ => (),
                    }
                }
                drop(reader);
                if path.is_empty() {
                    continue;
                }
                thread_requests.lock().unwrap().push(path.clone());
                let endpoint = path.strip_prefix("/liveclientdata/").unwrap_or("");
                let (status, body) = routes
                    .get(endpoint)
                    .cloned()
                    .unwrap_or((404, String::new()));
                let headers = if status == 302 {
                    "Location: https://127.0.0.1:1/forbidden\r\n"
                } else {
                    ""
                };
                let response = format!(
                    "HTTP/1.1 {status} Test\r\n{headers}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = tls.write_all(response.as_bytes());
                let _ = tls.flush();
            }
        });
        Self {
            url,
            root,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    fn client(&self) -> LiveClient {
        LiveClient::with_root(self.url.clone(), self.root.as_bytes()).unwrap()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

#[test]
fn poll_real_tls_mock_with_recorded_riot_subset() {
    let server = MockServer::start(fixtures(), false);
    let snapshot = server.client().poll().unwrap();
    assert_eq!(snapshot.active_player().unwrap().champion_name, "Annie");
    assert_eq!(snapshot.stats.map_number, 11);
    assert_eq!(snapshot.events.events[0].event_name, "GameStart");
    assert_eq!(
        *server.requests.lock().unwrap(),
        [
            "/liveclientdata/activeplayername",
            "/liveclientdata/playerlist",
            "/liveclientdata/eventdata",
            "/liveclientdata/gamestats",
        ]
    );
}

#[test]
fn riot_root_is_parseable_and_rejects_a_different_ca() {
    LiveClient::new().unwrap();
    let server = MockServer::start(fixtures(), false);
    let mut client = LiveClient::with_root(server.url.clone(), RIOT_ROOT_PEM).unwrap();
    assert!(matches!(client.poll(), Err(ClientError::Http(_))));
    assert!(server.requests.lock().unwrap().is_empty());
}

#[test]
fn hostname_exception_does_not_disable_expiration_validation() {
    let server = MockServer::start(fixtures(), true);
    assert!(matches!(server.client().poll(), Err(ClientError::Http(_))));
    assert!(server.requests.lock().unwrap().is_empty());
}

#[test]
fn errors_do_not_publish_a_partial_snapshot() {
    for (status, body) in [(503, "{}"), (200, "{broken"), (200, "{}"), (302, "{}")] {
        let mut routes = fixtures();
        routes.insert("playerlist", (status, body.into()));
        let server = MockServer::start(routes, false);
        assert!(server.client().poll().is_err());
        assert_eq!(server.requests.lock().unwrap().len(), 2);
    }
}

#[test]
fn response_size_and_negative_time_are_rejected() {
    for body in [
        " ".repeat(MAX_RESPONSE_BYTES as usize + 1),
        r#"{"gameMode":"CLASSIC","gameTime":-1,"mapName":"Map11","mapNumber":11}"#.into(),
    ] {
        let mut routes = fixtures();
        routes.insert("gamestats", (200, body));
        let server = MockServer::start(routes, false);
        assert!(matches!(
            server.client().poll(),
            Err(ClientError::InvalidData(_))
        ));
    }
}

#[test]
fn refused_connection_is_a_recoverable_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("https://{}", listener.local_addr().unwrap());
    drop(listener);
    let mut client = LiveClient::with_root(url, RIOT_ROOT_PEM).unwrap();
    assert!(matches!(client.poll(), Err(ClientError::Http(_))));
}

#[test]
fn cadence_throttles_failures_and_never_catches_up() {
    struct Broken(usize);
    impl LiveDataSource for Broken {
        fn poll(&mut self) -> Result<Snapshot, ClientError> {
            self.0 += 1;
            Err(ClientError::InvalidData("offline"))
        }
    }
    let mut poller = Poller::new(Broken(0));
    for t in [0, 1, 100] {
        assert!(poller.poll_if_due(Duration::from_secs(t)).unwrap().is_err());
        assert!(
            poller
                .poll_if_due(Duration::from_millis(t * 1000 + 999))
                .is_none()
        );
    }
    assert!(poller.poll_if_due(Duration::ZERO).is_none());
    assert_eq!(poller.source.0, 3);
}
