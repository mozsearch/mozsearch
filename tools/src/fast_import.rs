//! Our ends of git fast-import's input and output, for the history tools
//! (build-syntax-token-tree and build-timeline-tree), which write their repos
//! through it: threads write what we give it, and read what it answers, so
//! that we and it can work at the same time.

use std::io::{self, BufRead, Read, Write};
use std::process::{ChildStdin, ChildStdout};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

/// How many bytes we let `FastImportInput` queue for git fast-import.
const MAX_QUEUED_BYTES: usize = 256 << 20;
/// `FastImportInput` sends writes smaller than this together.
const SMALL_WRITES_BYTES: usize = 64 << 10;

/// Our end of git fast-import's input, which hands what we write to a thread
/// which writes it to the pipe, so that we can go on preparing the next
/// journals while git fast-import takes in the last ones.  (Writing to its
/// 64 KiB pipe ourselves, in the full firefox reblame we spent a fifth to a
/// quarter of our time waiting for room in it, and git fast-import a quarter
/// to a third of its time waiting for input.)  What we've written gets to git
/// fast-import before any requests we write later, and `FastImport::output`
/// sends everything on before we wait for an answer.
pub struct FastImportInput {
    /// Small writes, to send together.
    buffer: Vec<u8>,
    sender: Option<Sender<Vec<u8>>>,
    /// The bytes sent which the thread hasn't written yet, and whether it
    /// failed to write.
    queued: Arc<(Mutex<(usize, bool)>, Condvar)>,
    thread: Option<JoinHandle<io::Result<()>>>,
}

impl FastImportInput {
    pub fn new(mut stdin: ChildStdin) -> Self {
        let (sender, receiver) = channel::<Vec<u8>>();
        let queued = Arc::new((Mutex::new((0, false)), Condvar::new()));
        let thread_queued = queued.clone();
        let thread = thread::spawn(move || -> io::Result<()> {
            let (lock, condvar) = &*thread_queued;
            for chunk in receiver {
                let result = stdin.write_all(&chunk);
                let mut state = lock.lock().unwrap();
                state.0 -= chunk.len();
                state.1 |= result.is_err();
                condvar.notify_all();
                result?;
            }
            // Dropping `stdin` closes git fast-import's input.
            Ok(())
        });
        FastImportInput {
            buffer: vec![],
            sender: Some(sender),
            queued,
            thread: Some(thread),
        }
    }

    fn send(&mut self, chunk: Vec<u8>) {
        if chunk.is_empty() {
            return;
        }
        let (lock, condvar) = &*self.queued;
        let mut state = lock.lock().unwrap();
        while state.0 > 0 && state.0 + chunk.len() > MAX_QUEUED_BYTES && !state.1 {
            state = condvar.wait(state).unwrap();
        }
        state.0 += chunk.len();
        drop(state);
        // (This fails if the thread stopped because it failed to write.)
        self.sender
            .as_ref()
            .unwrap()
            .send(chunk)
            .expect("failed to write to git fast-import");
    }

    fn send_buffer(&mut self) {
        let chunk = std::mem::take(&mut self.buffer);
        self.send(chunk);
    }

    /// Write everything sent, and close git fast-import's input.
    pub fn finish(&mut self) -> io::Result<()> {
        self.send_buffer();
        drop(self.sender.take());
        match self.thread.take() {
            Some(thread) => thread.join().unwrap(),
            None => Ok(()),
        }
    }
}

impl Write for FastImportInput {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() >= SMALL_WRITES_BYTES {
            self.send_buffer();
            self.send(buf.to_vec());
        } else {
            self.buffer.extend_from_slice(buf);
            if self.buffer.len() >= SMALL_WRITES_BYTES {
                self.send_buffer();
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.send_buffer();
        Ok(())
    }
}

/// Our end of git fast-import's output, which a thread reads as it comes, so
/// that we can see whether it has answered without waiting for it (see
/// `FastImport::known_commit_rev`).
pub struct FastImportOutput {
    chunks: Receiver<Vec<u8>>,
    /// What we've taken from the thread and not read.
    buffer: Vec<u8>,
    pos: usize,
}

impl FastImportOutput {
    pub fn new(mut stdout: ChildStdout) -> Self {
        let (sender, chunks) = channel();
        thread::spawn(move || {
            let mut chunk = vec![0; 64 << 10];
            // (Until git fast-import exits, or we stop reading.)
            while let Ok(len @ 1..) = stdout.read(&mut chunk) {
                if sender.send(chunk[..len].to_vec()).is_err() {
                    break;
                }
            }
        });
        FastImportOutput {
            chunks,
            buffer: vec![],
            pos: 0,
        }
    }

    /// Take what the thread has read so far, without waiting for more.
    pub fn take_available(&mut self) {
        while let Ok(chunk) = self.chunks.try_recv() {
            self.buffer.drain(..self.pos);
            self.pos = 0;
            self.buffer.extend_from_slice(&chunk);
        }
    }

    /// Whether what we've taken has a whole line we haven't read.
    pub fn has_line(&self) -> bool {
        self.buffer[self.pos..].contains(&b'\n')
    }
}

impl Read for FastImportOutput {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let len = available.len().min(buf.len());
        buf[..len].copy_from_slice(&available[..len]);
        self.consume(len);
        Ok(len)
    }
}

impl BufRead for FastImportOutput {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.pos == self.buffer.len() {
            // (Nothing more once git fast-import exits.)
            if let Ok(chunk) = self.chunks.recv() {
                self.buffer = chunk;
                self.pos = 0;
            }
        }
        Ok(&self.buffer[self.pos..])
    }

    fn consume(&mut self, amount: usize) {
        self.pos += amount;
    }
}
