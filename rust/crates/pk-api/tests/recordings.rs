//! The recordings bucket client (T11 round 2, docs/adr/0015) against the S3 stand-in (`tests/support/s3_standin.rs`:
//! a local S3-compatible server that verifies the `SigV4` signature of every presigned URL, so the signing is checked,
//! and enforces R2's multipart rules). No database is needed here; the routes over a database are in `worker.rs`.
//!
//! What it proves: a recording that fits one part is one PUT and a longer one a multipart upload whose parts are all the
//! same size but the last; the bytes come back whole; a body cut short, stalled, or refused part-way aborts its
//! multipart upload and keeps nothing; a 5xx is tried again and a lasting one is a store error that names no URL, key
//! or bucket; a wrong secret or bucket is refused; only keys this service made are ever read or deleted; a listing
//! pages; and memory while a long recording streams through stays within a few parts, not the recording.

use std::io;
use std::time::Duration;

use bytes::Bytes;
use futures::{Stream, StreamExt as _, stream};
use pk_api::config::RecConfig;
use pk_api::recordings::{KEY_PREFIX, PART, RecError, Recordings, new_key};

#[path = "support/s3_standin.rs"]
mod s3_standin;
use s3_standin::{Options, Standin};

const IDLE: Duration = Duration::from_secs(5);

fn cfg(s: &Standin) -> RecConfig {
    RecConfig {
        endpoint: s.endpoint.clone(),
        bucket: s.bucket.clone(),
        key_id: s.key_id.clone(),
        secret: s.secret.clone(),
        region: "auto".into(),
    }
}

async fn start() -> (Standin, Recordings) {
    let s = Standin::start(Options::default()).await;
    let r = Recordings::new(&cfg(&s)).expect("client");
    (s, r)
}

fn pattern(n: usize) -> Vec<u8> {
    (0..n).map(|i| ((i * 31 + 7) % 251) as u8).collect()
}

/// `data` as a body arriving in `chunk`-byte pieces.
#[allow(clippy::needless_pass_by_value)] // callers hand over a throwaway vector
fn body(data: Vec<u8>, chunk: usize) -> impl Stream<Item = Result<Bytes, io::Error>> + Unpin {
    let pieces: Vec<Result<Bytes, io::Error>> = data.chunks(chunk).map(|c| Ok(Bytes::copy_from_slice(c))).collect();
    stream::iter(pieces)
}

async fn read_back(r: &Recordings, key: &str) -> Vec<u8> {
    let mut down = r.get(key).await.expect("get").expect("the object is there");
    let mut out = Vec::new();
    while let Some(b) = down.stream.next().await {
        out.extend_from_slice(&b.expect("a clean stream"));
    }
    assert_eq!(out.len() as u64, down.len, "the stream is as long as the answer said");
    out
}

#[tokio::test]
async fn a_recording_of_one_part_is_one_put_and_comes_back_whole() {
    let (s, r) = start().await;
    let key = new_key();
    let data = pattern(1_000_000);
    let done = r
        .put_stream(&key, body(data.clone(), 64 * 1024), IDLE)
        .await
        .expect("put");
    assert_eq!((done.bytes, done.parts), (1_000_000, 1));
    assert_eq!((s.count("put"), s.count("start"), s.count("part")), (1, 0, 0));
    assert_eq!(s.refused(), 0, "every signature the stand-in checked was good");
    assert_eq!(read_back(&r, &key).await, data);
    // exactly one part is still one PUT: a part goes only once more has arrived behind it
    let key2 = new_key();
    r.put_stream(&key2, body(pattern(PART), 1 << 20), IDLE)
        .await
        .expect("put");
    assert_eq!((s.count("put"), s.count("start")), (2, 0));
}

#[tokio::test]
async fn a_longer_recording_is_a_multipart_upload_of_equal_parts_and_comes_back_whole() {
    let (s, r) = start().await;
    let key = new_key();
    let data = pattern(2 * PART + 17);
    // odd chunk sizes, so no part boundary lands on a chunk boundary
    let done = r
        .put_stream(&key, body(data.clone(), 300_007), IDLE)
        .await
        .expect("put");
    assert_eq!((done.bytes, done.parts), ((2 * PART + 17) as u64, 3));
    assert_eq!(
        (s.count("start"), s.count("part"), s.completed(), s.open_uploads()),
        (1, 3, 1, 0)
    );
    assert_eq!(s.count("put"), 0);
    assert_eq!(s.objects(), vec![(key.clone(), 2 * PART + 17)]);
    assert_eq!(
        read_back(&r, &key).await,
        data,
        "the bytes come back whole and in order"
    );
    // one byte over a part: two parts, the last a single byte
    let key2 = new_key();
    let done = r
        .put_stream(&key2, body(pattern(PART + 1), 1 << 20), IDLE)
        .await
        .expect("put");
    assert_eq!(done.parts, 2);
}

#[tokio::test]
async fn a_body_cut_short_aborts_the_multipart_upload_and_keeps_nothing() {
    let (s, r) = start().await;
    let key = new_key();
    let mut pieces: Vec<Result<Bytes, io::Error>> = pattern(PART + PART / 2)
        .chunks(1 << 20)
        .map(|c| Ok(Bytes::copy_from_slice(c)))
        .collect();
    pieces.push(Err(io::Error::other("the phone lost its signal")));
    let e = r.put_stream(&key, stream::iter(pieces), IDLE).await.unwrap_err();
    assert!(matches!(e, RecError::Cut), "{e}");
    assert_eq!(s.count("start"), 1, "the upload had begun (a part had gone)");
    assert_eq!(
        (s.open_uploads(), s.aborted(), s.objects().len()),
        (0, 1, 0),
        "aborted, nothing stored"
    );
    // a short recording cut short never reached the bucket at all
    let e = r
        .put_stream(
            &new_key(),
            stream::iter(vec![Ok(Bytes::from_static(b"abc")), Err(io::Error::other("x"))]),
            IDLE,
        )
        .await
        .unwrap_err();
    assert!(matches!(e, RecError::Cut));
    assert_eq!((s.count("put"), s.objects().len()), (0, 0));
}

#[tokio::test]
async fn a_body_that_stalls_is_cut_short_after_the_idle_time() {
    let (s, r) = start().await;
    let first = stream::iter(vec![Ok::<_, io::Error>(Bytes::from(pattern(PART + 5)))]);
    let stalled = first.chain(stream::pending());
    let t = std::time::Instant::now();
    let e = r
        .put_stream(&new_key(), Box::pin(stalled), Duration::from_millis(300))
        .await
        .unwrap_err();
    assert!(matches!(e, RecError::Cut), "{e}");
    assert!(t.elapsed() < Duration::from_secs(4));
    assert_eq!((s.open_uploads(), s.aborted()), (0, 1));
}

#[tokio::test]
async fn a_5xx_is_tried_again_and_a_lasting_one_aborts_and_names_nothing_private() {
    let (s, r) = start().await;
    let data = pattern(PART + 100);
    // two failures of one part, then good: three tries are enough
    s.fail("part", 500, 2);
    r.put_stream(&new_key(), body(data.clone(), 1 << 20), IDLE)
        .await
        .expect("tried again");
    assert_eq!(s.count("part"), 4, "part 1 three times (two refused), part 2 once");
    s.heal();
    // it never gets better: the upload ends, the multipart upload is aborted, the text is only a status
    let before = s.objects().len();
    s.fail("part", 503, -1);
    let key = new_key();
    let e = r.put_stream(&key, body(data, 1 << 20), IDLE).await.unwrap_err();
    let text = e.to_string();
    assert!(matches!(e, RecError::Store(_)), "{text}");
    assert!(text.contains("503"), "{text}");
    for secret in [
        s.endpoint.as_str(),
        s.bucket.as_str(),
        s.key_id.as_str(),
        s.secret.as_str(),
        key.as_str(),
        "X-Amz",
        "http",
    ] {
        assert!(!text.contains(secret), "the error leaks {secret:?}: {text}");
    }
    assert_eq!(
        (s.open_uploads(), s.objects().len()),
        (0, before),
        "aborted, nothing new"
    );
    s.heal();
    // a 4xx is final at once (no retries)
    s.fail("put", 400, -1);
    let n = s.count("put");
    assert!(r.put_stream(&new_key(), body(pattern(10), 10), IDLE).await.is_err());
    assert_eq!(s.count("put"), n + 1);
}

#[tokio::test]
async fn a_complete_that_the_bucket_refuses_is_an_error_and_the_upload_is_aborted() {
    let (s, r) = start().await;
    s.fail("complete", 500, -1);
    let e = r
        .put_stream(&new_key(), body(pattern(PART + 9), 1 << 20), IDLE)
        .await
        .unwrap_err();
    assert!(matches!(e, RecError::Store(_)));
    assert_eq!((s.objects().len(), s.open_uploads(), s.aborted()), (0, 0, 1));
}

#[tokio::test]
async fn a_wrong_secret_or_bucket_is_refused_and_a_good_one_is_not() {
    let s = Standin::start(Options::default()).await;
    let good = Recordings::new(&cfg(&s)).expect("client");
    good.check().await.expect("the bucket answers a good token");
    let mut bad = cfg(&s);
    bad.secret.push('x'); // the right one, one character off
    let e = Recordings::new(&bad).expect("client").check().await.unwrap_err();
    assert!(e.to_string().contains("403"), "{e}");
    assert!(s.refused() >= 1, "the stand-in saw a signature that does not match");
    let mut other = cfg(&s);
    other.bucket = "another-bucket".into();
    let e = Recordings::new(&other).expect("client").check().await.unwrap_err();
    assert!(e.to_string().contains("404"), "{e}");
    // an upload with the wrong secret stores nothing
    let r = Recordings::new(&bad).expect("client");
    assert!(r.put_stream(&new_key(), body(pattern(10), 10), IDLE).await.is_err());
    assert_eq!(s.objects().len(), 0);
}

#[tokio::test]
async fn a_missing_object_is_none_a_missing_delete_is_fine_and_a_foreign_key_is_never_touched() {
    let (s, r) = start().await;
    assert!(r.get(&new_key()).await.expect("get").is_none());
    r.delete(&new_key())
        .await
        .expect("deleting what is not there is deleted");
    // a key this service did not make: no request is even sent
    s.plant("elsewhere/x", b"not a recording");
    let seen = s.ops().len();
    assert!(r.get("elsewhere/x").await.expect("get").is_none());
    assert!(r.delete("elsewhere/x").await.is_err());
    assert!(r.delete("rec/../elsewhere/x").await.is_err());
    assert_eq!(s.ops().len(), seen, "nothing was sent");
    assert_eq!(s.objects().len(), 1);
    // a lasting error on delete is reported
    let key = new_key();
    r.put_stream(&key, body(pattern(10), 10), IDLE).await.expect("put");
    s.fail("delete", 500, -1);
    assert!(r.delete(&key).await.is_err());
    s.heal();
    r.delete(&key).await.expect("delete");
    assert!(r.get(&key).await.expect("get").is_none());
}

#[tokio::test]
async fn a_listing_pages_and_gives_each_objects_age() {
    let s = Standin::start(Options {
        page: 2,
        ..Options::default()
    })
    .await;
    let r = Recordings::new(&cfg(&s)).expect("client");
    for i in 0..5 {
        s.plant(&format!("{KEY_PREFIX}k{i}"), b"x");
    }
    s.plant("other/skip", b"x");
    s.age(&format!("{KEY_PREFIX}k3"), 7200.0);
    let got = r.list().await.expect("list");
    assert_eq!(s.count("list"), 3, "five keys, two to a page");
    let keys: Vec<&str> = got.iter().map(|l| l.key.as_str()).collect();
    assert_eq!(
        keys,
        ["rec/k0", "rec/k1", "rec/k2", "rec/k3", "rec/k4"],
        "only rec/, in order"
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let age = |k: &str| {
        now - got
            .iter()
            .find(|l| l.key == k)
            .and_then(|l| l.modified)
            .expect("a time")
    };
    assert!(age("rec/k3") > 7000.0 && age("rec/k0") < 60.0);
}

/// The memory of this process in bytes (the working set).
fn rss() -> u64 {
    let pid = sysinfo::get_current_pid().expect("pid");
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    sys.process(pid).map_or(0, sysinfo::Process::memory)
}

/// A recording of 384 MiB goes through the client into a stand-in that keeps only sizes. The process (client and
/// stand-in together) must not grow by the recording: a part is buffered, sent, dropped. Red arm: a client that
/// collected the body before sending (every chunk kept) grows by the whole 384 MiB and fails the bound.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn memory_while_a_long_recording_streams_is_a_few_parts_not_the_recording() {
    const TOTAL: usize = 384 * 1024 * 1024;
    const CHUNK: usize = 256 * 1024;
    let s = Standin::start(Options {
        sink: true,
        ..Options::default()
    })
    .await;
    let r = Recordings::new(&cfg(&s)).expect("client");
    let chunk = Bytes::from(pattern(CHUNK));
    let body = Box::pin(stream::unfold(0_usize, move |sent| {
        let chunk = chunk.clone();
        async move { (sent < TOTAL).then(|| (Ok::<_, io::Error>(chunk), sent + CHUNK)) }
    }));
    let base = rss();
    let peak = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(base));
    let watcher = {
        let peak = peak.clone();
        tokio::spawn(async move {
            loop {
                peak.fetch_max(rss(), std::sync::atomic::Ordering::Relaxed);
                tokio::time::sleep(Duration::from_millis(15)).await;
            }
        })
    };
    let key = new_key();
    let done = r.put_stream(&key, body, Duration::from_secs(60)).await.expect("put");
    watcher.abort();
    assert_eq!(done.bytes, TOTAL as u64);
    assert_eq!(
        done.parts,
        i64::try_from(TOTAL / PART).unwrap(),
        "{} parts of {PART}",
        TOTAL / PART
    );
    let grew = peak.load(std::sync::atomic::Ordering::Relaxed).saturating_sub(base);
    eprintln!(
        "MEMORY streamed {} MiB, the process grew by {} MiB at most (part = {} MiB)",
        TOTAL >> 20,
        grew >> 20,
        PART >> 20
    );
    assert!(
        grew < 160 * 1024 * 1024,
        "grew by {} MiB while streaming {} MiB",
        grew >> 20,
        TOTAL >> 20
    );
}

#[tokio::test]
async fn the_start_up_check_is_a_listing_of_one_object_and_never_a_bucket_head() {
    let s = Standin::start(Options::default()).await;
    let r = Recordings::new(&cfg(&s)).expect("client");
    r.check().await.expect("a listing is allowed to this token");
    assert_eq!((s.count("list"), s.count("head")), (1, 0));
    // red arm: a check that asks with HEAD is refused by this stand-in the way Cloudflare refuses an Object Read & Write token
    s.plant("rec/one", b"x");
    s.plant("rec/two", b"x");
    r.check().await.expect("two objects present: still one listing page");
    assert_eq!(s.count("list"), 2);
    s.fail("list", 403, -1);
    assert!(
        r.check().await.is_err(),
        "a token that may not list keeps the pod unready"
    );
}

/// A body that runs past its declared length ends the upload at once: the rest is never read, nothing is kept, and a
/// multipart upload already begun is aborted. Red arm: without the check the whole body is read and stored.
#[tokio::test]
async fn a_body_past_its_content_length_is_stopped_as_soon_as_it_passes_it() {
    let (s, r) = start().await;
    let polled = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = polled.clone();
    // 40 chunks of 1 MiB offered; the declared size is 10 MiB + 5
    let body = Box::pin(stream::unfold(0_usize, move |i| {
        let counter = counter.clone();
        async move {
            (i < 40).then(|| {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                (Ok::<_, io::Error>(Bytes::from(vec![1_u8; 1 << 20])), i + 1)
            })
        }
    }));
    let e = r
        .put_stream_within(&new_key(), body, IDLE, 10 * 1024 * 1024 + 5)
        .await
        .unwrap_err();
    assert!(matches!(e, RecError::TooLong), "{e}");
    assert_eq!(
        polled.load(std::sync::atomic::Ordering::SeqCst),
        11,
        "stopped at the chunk that passed the size"
    );
    assert_eq!((s.objects().len(), s.open_uploads(), s.aborted()), (0, 0, 1));
    // exactly its size is fine
    let ok = r
        .put_stream_within(&new_key(), body_of(3 << 20), IDLE, 3 << 20)
        .await
        .expect("put");
    assert_eq!(ok.bytes, 3 << 20);
}

fn body_of(n: usize) -> impl Stream<Item = Result<Bytes, io::Error>> + Unpin {
    body(pattern(n), 1 << 20)
}
