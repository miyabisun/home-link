use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_home-link"));
    command
        .env_remove("PORT")
        .env("LOG_LEVEL", "off")
        .env("DATABASE_PATH", ":memory:");
    command
}

#[test]
fn port_selects_http_listener_and_legacy_address_is_ignored() {
    // An occupied legacy address makes accidental use of APP_BIND_ADDR observable.
    let legacy = TcpListener::bind("127.0.0.1:0").unwrap();
    for _ in 0..2 {
        let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = reservation.local_addr().unwrap();
        drop(reservation);
        let mut command = command();

        let mut server = Server(
            command
                .env("PORT", address.port().to_string())
                .env("APP_BIND_ADDR", legacy.local_addr().unwrap().to_string())
                .stdout(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            assert!(
                server.0.try_wait().unwrap().is_none(),
                "server exited early"
            );
            if let Ok(stream) = TcpStream::connect(address) {
                break stream;
            }
            assert!(Instant::now() < deadline, "PORT listener did not start");
            thread::sleep(Duration::from_millis(20));
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
            .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.ends_with("\r\n\r\nok\n"), "{response}");
    }
}

#[test]
fn invalid_port_fails_startup_with_an_explicit_error() {
    for port in [
        "",
        "0",
        "65536",
        "abc",
        "-1",
        "+3000",
        " 3000",
        "3000 ",
        "127.0.0.1:3000",
    ] {
        let mut command = command();

        let mut server = Server(
            command
                .env("PORT", port)
                .env("APP_BIND_ADDR", "invalid-legacy-address")
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = server.0.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "invalid PORT {port:?} did not fail"
            );
            thread::sleep(Duration::from_millis(10));
        };
        assert!(!status.success(), "invalid PORT {port:?} succeeded");
        let mut error = String::new();
        server
            .0
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut error)
            .unwrap();
        assert!(error.contains("PORT"), "{port:?}: {error}");
    }
}
