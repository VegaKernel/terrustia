//! The web admin panel's foundation, exercised over a real TCP connection — not an in-process
//! mock. Player list, kick/ban, world screen and the rest are follow-up work (see `panel/mod.rs`'s
//! module doc); this covers what exists today: static asset serving, the unclaimed/claim flow, an
//! ordinary login, and that `/api/status` actually requires a valid session.

use terrustia::{
    config::Config,
    game::{GameServer, ServerEvent},
    panel,
    world::worldgen,
};
use tokio::sync::{mpsc, oneshot};

#[tokio::test]
async fn static_assets_and_the_unclaimed_flow_work_over_a_real_socket() {
    // Bind the real ephemeral listener ourselves so the address is known before the panel task
    // exists, rather than needing `panel::run` to report back what it chose.
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = probe.local_addr().unwrap();
    drop(probe); // release the port; the panel binds it again immediately below

    let config = Config {
        world_width: 800,
        world_height: 600,
        motd: String::new(),
        panel_enabled: true,
        panel_listen: addr,
        ..Config::default()
    };

    let world = worldgen::generate(
        config.world_width,
        config.world_height,
        config.world_name.clone(),
        7,
    );
    let (tx, rx) = mpsc::channel::<ServerEvent>(1024);
    tokio::spawn(GameServer::new(config.clone(), world).run(rx));
    let _panel = panel::run(config, tx.clone())
        .await
        .expect("panel should bind its configured loopback address");

    let base = format!("http://{addr}");
    let client = reqwest_lite::Client::new();

    // The app shell is served for the root path, over real HTTP.
    let index = client.get(&base).await;
    assert!(
        index.contains("<div id=\"app\">") || index.contains("id=\"app\""),
        "expected the built index.html to be served at /, got: {index}"
    );

    // Freshly generated world, no accounts registered yet.
    let unclaimed = client.get(&format!("{base}/api/unclaimed")).await;
    assert!(
        unclaimed.contains("\"unclaimed\":true"),
        "a fresh world has no accounts yet: {unclaimed}"
    );

    // The real claim token, the way an operator would actually get it: read off the game task's
    // own state, the same value `announce_claim_token` printed to the console.
    let (reply, rx) = oneshot::channel();
    tx.send(ServerEvent::PanelAuthLookup {
        name: String::new(),
        reply,
    })
    .await
    .unwrap();
    let lookup = rx.await.unwrap();
    let token = lookup
        .claim_token
        .expect("run() announces a token immediately on start");

    // Wrong token is refused.
    let (status, _body) = client
        .post_json(
            &format!("{base}/api/login"),
            r#"{"name":"admin","password":"correcthorsebatterystaple","claim_token":"wrong"}"#,
        )
        .await;
    assert_eq!(status, 401, "a wrong claim token must not claim the server");

    // The right token claims it and returns a real session.
    let (status, body) = client
        .post_json(
            &format!("{base}/api/login"),
            &format!(
                r#"{{"name":"admin","password":"correcthorsebatterystaple","claim_token":"{token}"}}"#
            ),
        )
        .await;
    assert_eq!(status, 200, "the real claim token should succeed: {body}");
    let session = extract_session(&body);

    // /api/status refuses no session at all.
    let (status, _) = client.get_status(&format!("{base}/api/status"), None).await;
    assert_eq!(status, 401, "status must require a session");

    // ...and accepts the one just issued, with real, non-placeholder data.
    let (status, body) = client
        .get_status(&format!("{base}/api/status"), Some(&session))
        .await;
    assert_eq!(
        status, 200,
        "a freshly issued session should be accepted: {body}"
    );
    assert!(body.contains("\"world_name\""));
    assert!(
        body.contains("\"unclaimed\":false"),
        "claiming should have flipped this: {body}"
    );

    // The server is claimed now — signing in again with a claim_token field present is simply
    // ignored, and the real password is what's checked.
    let (status, body) = client
        .post_json(
            &format!("{base}/api/login"),
            r#"{"name":"admin","password":"wrongpassword"}"#,
        )
        .await;
    assert_eq!(status, 401, "wrong password must be refused: {body}");

    let (status, body) = client
        .post_json(
            &format!("{base}/api/login"),
            r#"{"name":"admin","password":"correcthorsebatterystaple"}"#,
        )
        .await;
    assert_eq!(status, 200, "the real password should sign in: {body}");
}

fn extract_session(body: &str) -> String {
    let key = "\"session\":\"";
    let start = body
        .find(key)
        .expect("login response should carry a session")
        + key.len();
    let end = body[start..].find('"').unwrap() + start;
    body[start..end].to_string()
}

/// A minimal HTTP/1.1 client over a raw TCP socket — this workspace has no HTTP client dependency
/// (the game protocol has nothing to do with HTTP), and pulling one in just for three test
/// requests would be a heavier dependency than the thing it is testing.
mod reqwest_lite {
    use std::time::Duration;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
    };

    pub struct Client;

    impl Client {
        pub fn new() -> Self {
            Client
        }

        pub async fn get(&self, url: &str) -> String {
            let (_status, body) = self.request("GET", url, None, None).await;
            body
        }

        pub async fn get_status(&self, url: &str, session: Option<&str>) -> (u16, String) {
            self.request("GET", url, None, session).await
        }

        pub async fn post_json(&self, url: &str, json: &str) -> (u16, String) {
            self.request("POST", url, Some(json), None).await
        }

        async fn request(
            &self,
            method: &str,
            url: &str,
            json_body: Option<&str>,
            session: Option<&str>,
        ) -> (u16, String) {
            let (host_port, path) = split_url(url);
            let mut stream =
                tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(&host_port))
                    .await
                    .expect("connect timed out")
                    .expect("connect failed");

            let mut request =
                format!("{method} {path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n");
            if let Some(session) = session {
                request.push_str(&format!("Authorization: Bearer {session}\r\n"));
            }
            match json_body {
                Some(json) => {
                    request.push_str("Content-Type: application/json\r\n");
                    request.push_str(&format!("Content-Length: {}\r\n\r\n{json}", json.len()));
                }
                None => request.push_str("\r\n"),
            }

            stream
                .write_all(request.as_bytes())
                .await
                .expect("write failed");
            let mut raw = Vec::new();
            tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut raw))
                .await
                .expect("read timed out")
                .expect("read failed");
            let raw = String::from_utf8_lossy(&raw);

            let mut parts = raw.splitn(2, "\r\n\r\n");
            let head = parts.next().unwrap_or_default();
            let body = parts.next().unwrap_or_default().to_string();
            let status = head
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|code| code.parse().ok())
                .unwrap_or(0);
            (status, body)
        }
    }

    fn split_url(url: &str) -> (String, String) {
        let rest = url.strip_prefix("http://").unwrap_or(url);
        match rest.find('/') {
            Some(i) => (rest[..i].to_string(), rest[i..].to_string()),
            None => (rest.to_string(), "/".to_string()),
        }
    }
}
