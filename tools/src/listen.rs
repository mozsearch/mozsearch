//! The web-server's and the pipeline-server's ports, when they restart: the
//! previous server can still hold its port for a moment after it's been told
//! to exit, since the kernel closes an exiting process's sockets only after
//! tearing down its memory, which a big process's takes a while (a process of
//! 6 GB kept its port for 0.43 s after SIGTERM locally, while
//! web-server-run.sh waited 0.1 s after its `pkill`s; the pipeline-server maps
//! tens of GB of index files).  And on config8's web server, the
//! pipeline-server's port was still in use as the new one started, right
//! after `systemctl stop` of the old one's service (pipeline-server-run.sh),
//! in both restarts of 2026-11-10.  A server that couldn't bind its port
//! panicked, and its supervisor (with-auto-restart.sh) restarted it a second
//! later, with the panic's backtrace and the restart in its error log, and
//! after 10 failures, an email; so a server waits for its port (see
//! `bind_waiting`), and fails as before if it stays in use.

use std::io::{self, ErrorKind};
use std::time::{Duration, Instant};

use tokio::net::TcpListener;

/// How long a server waits for its port to be free.
pub const PORT_WAIT: Duration = Duration::from_secs(30);
/// How often it tries again.
const RETRY_PERIOD: Duration = Duration::from_millis(100);

/// Listen on `addr` (ex: "0.0.0.0:8002"), waiting up to `PORT_WAIT` for it if
/// it's in use; and if it can't, panic, as servers did, so that its supervisor
/// restarts it (and says so).
pub async fn bind_waiting(addr: &str) -> TcpListener {
    bind_waiting_for(addr, PORT_WAIT)
        .await
        .unwrap_or_else(|e| panic!("Couldn't listen on {addr}: {e}"))
}

/// Listen on `addr`, waiting up to `wait` for it if it's in use, and saying
/// so in the error log.
pub async fn bind_waiting_for(addr: &str, wait: Duration) -> io::Result<TcpListener> {
    let start = Instant::now();
    let mut waited = false;
    loop {
        match TcpListener::bind(addr).await {
            Ok(listener) => {
                if waited {
                    eprintln!(
                        "{addr} was free after {:.2} s",
                        start.elapsed().as_secs_f64()
                    );
                }
                return Ok(listener);
            }
            Err(e) if e.kind() == ErrorKind::AddrInUse && start.elapsed() < wait => {
                if !waited {
                    eprintln!(
                        "{addr} is in use (ex: by the previous server, still exiting); waiting up to {} s for it",
                        wait.as_secs()
                    );
                    waited = true;
                }
                tokio::time::sleep(RETRY_PERIOD).await;
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_waits_for_the_port() {
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = taken.local_addr().unwrap().to_string();
        // (Freed a moment later, as by a server exiting.)
        let freer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(taken);
        });
        let start = Instant::now();
        let listener = bind_waiting_for(&addr, Duration::from_secs(10)).await;
        assert!(listener.is_ok(), "{listener:?}");
        assert!(start.elapsed() >= Duration::from_millis(250));
        freer.join().unwrap();
    }

    #[tokio::test]
    async fn test_gives_up() {
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = taken.local_addr().unwrap().to_string();
        let start = Instant::now();
        let err = bind_waiting_for(&addr, Duration::from_millis(300))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AddrInUse);
        assert!(start.elapsed() >= Duration::from_millis(300));
        drop(taken);
    }
}
