//! Two seats, two sockets, one whole match through the deployed relay.
//!
//! Composition root. No behavior: the seat is `crates/seat` and the state machine behind
//! it is `crates/coordinator`.
//!
//! This is the first thing in the project that sends a message across a process boundary.
//! It exists before any browser client on purpose: a page that fails to play a match has
//! six suspects — the socket, the frame format, the seat, the wasm build, the Lambda and
//! the page — and this reduces them to two. When the browser client later misbehaves, a
//! socket that already plays a match means the fault is in the page.
//!
//!   cargo run -p probe -- wss://<api>.execute-api.<region>.amazonaws.com/live

use std::sync::mpsc;
use std::time::{Duration, Instant};

use protocol::Account;
use relay::MatchId;
use seat::{Ai, Seated, Stopped};
use tungstenite::{Message as Frame, connect};

/// How long one seat will wait for the match to finish before giving up.
const PATIENCE: Duration = Duration::from_secs(120);

fn main() {
    // Both aws-lc-rs and ring reach this binary through the dependency graph, so rustls
    // declines to guess which one to use and says so at the first handshake. Choosing it
    // here is the composition root's job, and this is the composition root.
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("no crypto provider should be installed yet");

    let url = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: probe wss://host/stage");
        std::process::exit(2);
    });
    // A nonce per run, so each run hosts its own match rather than being answered with
    // the previous run's. Idempotency is a property worth having and not one worth
    // tripping over here.
    let nonce = format!(
        "probe-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_millis())
            .unwrap_or_default()
    );

    let (tell, told) = mpsc::channel::<MatchId>();
    let host_url = url.clone();
    let host_nonce = nonce.clone();

    let host = std::thread::spawn(move || {
        let seated = Seated::hosting(
            Account::Discord("probe-host".to_string()),
            "Host",
            "red",
            host_nonce,
            Box::new(Ai),
        );
        play("host", &host_url, seated, Some(tell))
    });

    // If the host never names a match it has already stopped for a reason, and that
    // reason is the only useful thing this program can say. Panicking on the channel
    // would report `Disconnected`, which says nothing — the first version did, and the
    // actual answer was a version mismatch the relay had reported politely.
    let match_id = match told.recv_timeout(PATIENCE) {
        Ok(match_id) => match_id,
        Err(_) => {
            let host = host.join().expect("the host thread");
            report("host", &host);
            eprintln!("[probe] the host never named a match, so there was nothing to join");
            std::process::exit(1);
        }
    };
    println!("[probe] match {}", match_id.0);

    let guest = std::thread::spawn(move || {
        let seated = Seated::joining(
            Account::Discord("probe-guest".to_string()),
            "Guest",
            "blue",
            match_id,
            Box::new(Ai),
        );
        play("guest", &url, seated, None)
    });

    let host = host.join().expect("the host thread");
    let guest = guest.join().expect("the guest thread");
    report("host", &host);
    report("guest", &guest);

    let failed = [&host, &guest].iter().any(|outcome| !outcome.ok());
    if failed {
        std::process::exit(1);
    }
    println!("[probe] a whole match played over a real socket");
}

/// What one seat saw.
struct Played {
    events: usize,
    complaints: Vec<coordinator::Wrong>,
    stopped: Option<Stopped>,
    over: bool,
    /// How the relay said the match ended, which is not the same as this seat working it
    /// out. The relay writes no resolution event and may not derive one, so this frame
    /// coming back is the proof that it learned the match was finished — and so that it
    /// will stop offering the match in the lobby.
    ending: Option<seat::Ending>,
}

impl Played {
    fn ok(&self) -> bool {
        self.over
            && self.ending == Some(seat::Ending::Complete)
            && self.complaints.is_empty()
            && !matches!(self.stopped, Some(Stopped::Refused(_)))
    }
}

fn report(who: &str, played: &Played) {
    println!(
        "[{who}] {} events, {} complaints, over: {}, ending: {:?}",
        played.events,
        played.complaints.len(),
        played.over,
        played.ending
    );
    for complaint in &played.complaints {
        println!("[{who}] COMPLAINT {complaint:?}");
    }
    if let Some(stopped) = &played.stopped {
        println!("[{who}] stopped: {stopped}");
    }
}

/// Connect, open, and answer frames until the match ends.
fn play(who: &str, url: &str, mut seated: Seated, tell: Option<mpsc::Sender<MatchId>>) -> Played {
    let (mut socket, _) = connect(url).unwrap_or_else(|error| {
        // The URL, the TLS and the gateway are all in play here, so say which one.
        panic!("[{who}] could not open a socket to {url}: {error}")
    });
    socket
        .send(Frame::Text(seated.opening().into()))
        .expect("the opening frame should send");

    let began = Instant::now();
    let mut told = false;
    let mut started = false;
    let mut stopped = None;

    // Until the relay says the match ended, not until this seat works it out. A seat
    // derives the ending and says so in the same breath, and stopping at its own verdict
    // would close the socket before the `complete` frame came back — which is the one
    // thing that proves the relay heard. The frames this seat sends go out inside the loop
    // body, so there is nothing buffered to lose either way.
    while seated.ending().is_none() && began.elapsed() < PATIENCE {
        let frame = match socket.read() {
            Ok(Frame::Text(text)) => text.to_string(),
            // A close or a ping is the transport's business, not a seat's.
            Ok(Frame::Close(_)) => break,
            Ok(_) => continue,
            Err(error) => {
                println!("[{who}] socket: {error}");
                break;
            }
        };
        match seated.receive(&frame) {
            Ok(answers) => {
                for answer in answers {
                    if socket.send(Frame::Text(answer.into())).is_err() {
                        break;
                    }
                }
            }
            Err(why) => {
                stopped = Some(why);
                break;
            }
        }

        // The host tells the guest which match to join, once, as soon as it knows.
        if !told
            && let Some(tell) = &tell
            && let Some(match_id) = seated.match_id()
        {
            let _ = tell.send(match_id.clone());
            told = true;
        }

        // And starts it once both seats are taken. Only the host may; the guest's
        // `start` is never sent because it has no reason to.
        if !started
            && tell.is_some()
            && seated.seated() >= 2
            && let Some(start) = seated.start()
        {
            socket
                .send(Frame::Text(start.into()))
                .expect("the start frame should send");
            started = true;
        }
    }

    let _ = socket.close(None);
    Played {
        events: seated.stream().len(),
        complaints: seated.complaints().to_vec(),
        stopped,
        over: seated.over(),
        ending: seated.ending(),
    }
}
