//! Pull a still frame from a camera.
//!
//! The issuing flow does not care where an image comes from — it takes bytes.
//! That boundary is why a fixed camera can be added without touching
//! recognition, matching or the FIFO rules (M7 architecture: CaptureSource).
//!
//! * `http`   GET a snapshot URL. What virtually every IP camera exposes
//!   (`/snapshot.jpg`, `/cgi-bin/snapshot.cgi`, …). Optional basic auth.
//! * `raw`    Open a socket, optionally send a trigger string, read until the
//!   peer closes or the JPEG end marker appears. For cameras with a bare TCP
//!   command port and no HTTP layer.
//! * `folder` Take the newest image in a directory. For industrial smart
//!   cameras (Hikrobot MV-SC and the like) that speak GigE Vision or a vendor
//!   SDK, but whose own software already saves every shot to disk. The share
//!   is the interface; we do not reimplement their SDK.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use base64::Engine;
use serde::{Deserialize, Serialize};

pub const DEFAULT_TIMEOUT: f64 = 8.0;
pub const MAX_BYTES: usize = 12 * 1024 * 1024;

const JPEG_START: &[u8] = b"\xff\xd8";
const JPEG_END: &[u8] = b"\xff\xd9";
const PNG_START: &[u8] = b"\x89PNG\r\n\x1a\n";
const BMP_START: &[u8] = b"BM";

pub const IMAGE_SUFFIXES: &[&str] = &["jpg", "jpeg", "png", "bmp"];

/// How long to let a file settle. The camera's own software is writing it,
/// and a still that is half on disk decodes as a grey band — which reads as a
/// bad photo rather than a race, so nobody thinks to look here.
const SETTLE_TRIES: usize = 15;
const SETTLE_WAIT: Duration = Duration::from_millis(200);

/// Older than this and the shot is not the box in front of you. Five minutes
/// covers walking the box over and getting the label straight, and is short
/// enough that the previous pallet's photo cannot stand in for this one.
pub const STALE_AFTER: f64 = 300.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraConfig {
    pub enabled: bool,
    /// `http` | `raw` | `folder`
    pub transport: String,
    pub host: String,
    pub port: u16,
    /// http only
    pub path: String,
    pub username: String,
    pub password: String,
    /// raw only: bytes to send before reading
    pub trigger: String,
    /// folder only: where the camera saves stills
    pub folder: String,
    pub timeout: f64,
}

impl Default for CameraConfig {
    fn default() -> Self {
        CameraConfig {
            enabled: false,
            transport: "http".into(),
            host: String::new(),
            port: 80,
            path: "/snapshot.jpg".into(),
            username: String::new(),
            password: String::new(),
            trigger: String::new(),
            folder: String::new(),
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

impl CameraConfig {
    pub fn url(&self) -> String {
        let path = if self.path.starts_with('/') { self.path.clone() } else { format!("/{}", self.path) };
        format!("http://{}:{}{}", self.host, self.port, path)
    }

    pub fn endpoint(&self) -> String {
        match self.transport.as_str() {
            "folder" => format!("file://{}", self.folder),
            "http" => self.url(),
            _ => format!("tcp://{}:{}", self.host, self.port),
        }
    }

    /// Whether the source has the one field it cannot work without.
    pub fn configured(&self) -> bool {
        if self.transport == "folder" { !self.folder.is_empty() } else { !self.host.is_empty() }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs_f64(if self.timeout > 0.0 { self.timeout } else { DEFAULT_TIMEOUT })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CameraError {
    /// Reaching the camera failed. The message is meant to be shown to whoever
    /// is standing in front of it, so it names the address and what went wrong.
    Failed(String),
    /// The newest image in the folder is too old to be the box in front of
    /// you. Not a fault: the camera is fine, it just has not taken a picture
    /// recently. The screen stays blank and waits, rather than showing a photo
    /// of whatever was scanned before lunch.
    Stale { message: String, age_seconds: f64, source_time: String },
}

impl std::fmt::Display for CameraError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CameraError::Failed(m) => f.write_str(m),
            CameraError::Stale { message, .. } => f.write_str(message),
        }
    }
}

impl std::error::Error for CameraError {}

fn fail<T>(message: impl Into<String>) -> Result<T, CameraError> {
    Err(CameraError::Failed(message.into()))
}

/// A captured frame, plus where it came from when that is knowable.
#[derive(Debug, Clone)]
pub struct Frame {
    pub data: Vec<u8>,
    /// folder source: file name and the camera's own save time — the moment
    /// the box was in front of the lens, which is what the operator checks.
    pub source: Option<SourceInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceInfo {
    pub name: String,
    pub age_seconds: f64,
    pub time: String,
}

/// Fetch one frame. Errors carry something actionable.
pub fn capture(config: &CameraConfig) -> Result<Frame, CameraError> {
    if config.transport == "folder" {
        if config.folder.is_empty() {
            return fail("尚未設定影像資料夾");
        }
        return capture_folder(config);
    }
    if config.host.is_empty() {
        return fail("尚未設定相機位址");
    }
    let data = match config.transport.as_str() {
        "http" => capture_http(config)?,
        "raw" => capture_raw(config)?,
        other => return fail(format!("未知的連線方式：{other}")),
    };
    Ok(Frame { data, source: None })
}

fn capture_http(config: &CameraConfig) -> Result<Vec<u8>, CameraError> {
    let agent: ureq::Agent =
        ureq::Agent::config_builder().timeout_global(Some(config.timeout())).http_status_as_error(false).build().into();
    let mut request = agent.get(&config.url());
    if !config.username.is_empty() {
        let token =
            base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", config.username, config.password));
        request = request.header("Authorization", &format!("Basic {token}"));
    }
    let addr = format!("{}:{}", config.host, config.port);
    let mut response = match request.call() {
        Ok(r) => r,
        Err(ureq::Error::Timeout(_)) => return fail(format!("相機 {addr} 逾時（{} 秒）", config.timeout)),
        Err(e) => return fail(format!("連不到相機 {addr} — {e}")),
    };
    let status = response.status().as_u16();
    if status >= 400 {
        return fail(format!("相機回應 HTTP {status}（{}）", config.url()));
    }
    let data = response.body_mut().with_config().limit(MAX_BYTES as u64 + 1).read_to_vec().or_else(|e| match e {
        // Over the limit: report as oversized rather than as a network fault.
        ureq::Error::BodyExceedsLimit(_) => Ok(vec![0; MAX_BYTES + 1]),
        ureq::Error::Timeout(_) => fail(format!("相機 {addr} 逾時（{} 秒）", config.timeout)),
        other => fail(format!("讀取相機 {addr} 失敗 — {other}")),
    })?;
    validate(data, config)
}

fn capture_raw(config: &CameraConfig) -> Result<Vec<u8>, CameraError> {
    let addr = format!("{}:{}", config.host, config.port);
    let timeout = config.timeout();
    let timed_out = || fail(format!("相機 {addr} 逾時（{} 秒）", config.timeout));
    let sockaddr = match addr.to_socket_addrs().map(|mut a| a.next()) {
        Ok(Some(a)) => a,
        Ok(None) => return fail(format!("連不到相機 {addr} — 找不到位址")),
        Err(e) => return fail(format!("連不到相機 {addr} — {e}")),
    };
    let mut stream = match TcpStream::connect_timeout(&sockaddr, timeout) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => return timed_out(),
        Err(e) => return fail(format!("連不到相機 {addr} — {e}")),
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    if !config.trigger.is_empty() {
        let trigger = unescape(&config.trigger);
        if let Err(e) = stream.write_all(trigger.as_bytes()) {
            return fail(format!("連不到相機 {addr} — {e}"));
        }
    }
    let mut data = Vec::new();
    let mut chunk = vec![0u8; 65536];
    while data.len() <= MAX_BYTES {
        let n = match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                // A frame that already arrived whole is fine; silence before
                // anything arrived is a timeout.
                if data.is_empty() {
                    return timed_out();
                }
                break;
            }
            Err(e) => return fail(format!("連不到相機 {addr} — {e}")),
        };
        data.extend_from_slice(&chunk[..n]);
        // Stop at the frame boundary rather than waiting for a close the
        // camera may never perform.
        if data.windows(2).rev().take(n + 1).any(|w| w == JPEG_END) {
            break;
        }
    }
    validate(data, config)
}

/// Settings forms hold `SNAP\r\n` typed literally; send what was meant.
fn unescape(text: &str) -> String {
    text.replace("\\r", "\r").replace("\\n", "\n").replace("\\t", "\t")
}

fn mtime(path: &Path) -> Option<SystemTime> {
    path.metadata().and_then(|m| m.modified()).ok()
}

fn local_iso(time: SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Local> = time.into();
    dt.format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// The newest image in the folder, and how many seconds old it is.
///
/// Age is returned rather than judged: a stale shot is not an error the way an
/// unreachable camera is. But it has to reach the screen, because silently
/// recognising an hour-old photo is how someone confirms an issue against a
/// box that is no longer there.
pub fn newest_image(config: &CameraConfig) -> Result<(PathBuf, SourceInfo), CameraError> {
    let folder = Path::new(&config.folder);
    if !folder.is_dir() {
        return fail(format!("找不到影像資料夾：{}", config.folder));
    }
    let entries = match std::fs::read_dir(folder) {
        Ok(e) => e,
        Err(e) => return fail(format!("讀不到影像資料夾 {} — {e}", config.folder)),
    };
    let newest = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| IMAGE_SUFFIXES.contains(&x.to_ascii_lowercase().as_str()))
        })
        .filter_map(|p| mtime(&p).map(|t| (p, t)))
        .max_by_key(|(_, t)| *t);
    let Some((path, time)) = newest else {
        return fail(format!("資料夾裡還沒有任何影像：{}", config.folder));
    };
    let age = SystemTime::now().duration_since(time).map_or(0.0, |d| d.as_secs_f64());
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    Ok((path, SourceInfo { name, age_seconds: age, time: local_iso(time) }))
}

fn capture_folder(config: &CameraConfig) -> Result<Frame, CameraError> {
    let (path, info) = newest_image(config)?;
    if info.age_seconds > STALE_AFTER {
        return Err(CameraError::Stale {
            message: format!(
                "最新的影像是 {} 分鐘前存的（{}），先拍一張再試。",
                (info.age_seconds / 60.0) as i64,
                info.name
            ),
            age_seconds: info.age_seconds,
            source_time: info.time,
        });
    }
    // Wait for the size to stop moving before reading, so a shot taken this
    // second is not read mid-write.
    let mut last: Option<u64> = None;
    for _ in 0..SETTLE_TRIES {
        let size = path.metadata().map(|m| m.len()).unwrap_or(0);
        if last == Some(size) && size > 0 {
            break;
        }
        last = Some(size);
        std::thread::sleep(SETTLE_WAIT);
    }
    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(e) => return fail(format!("讀不到 {} — {e}", info.name)),
    };
    Ok(Frame { data: validate(data, config)?, source: Some(info) })
}

/// A camera writing to disk may well save PNG or BMP; over the wire a still is
/// a JPEG in practice. Both get checked, so "not an image" stays a real error
/// rather than reaching recognition as bytes it cannot decode.
pub fn validate(data: Vec<u8>, config: &CameraConfig) -> Result<Vec<u8>, CameraError> {
    if data.is_empty() {
        return fail(format!("相機 {} 沒有回傳資料", config.endpoint()));
    }
    if data.len() > MAX_BYTES {
        return fail(format!("相機回傳超過 {}MB，可能不是單張影像", MAX_BYTES / 1024 / 1024));
    }
    let folder = config.transport == "folder";
    let magic: &[&[u8]] = if folder { &[JPEG_START, PNG_START, BMP_START] } else { &[JPEG_START] };
    if !magic.iter().any(|m| data.starts_with(m)) {
        // Naming what did arrive turns "it didn't work" into something
        // fixable — usually the path is an HTML error page or a stream.
        let head: String =
            String::from_utf8_lossy(&data[..data.len().min(80)]).trim().replace('\n', " ").chars().take(60).collect();
        let kinds = if folder { "JPEG／PNG／BMP" } else { "JPEG" };
        return fail(format!("讀到的不是 {kinds} 影像。開頭是：{head}"));
    }
    Ok(data)
}

/// Media type for an image's bytes, for recognition requests.
pub fn media_type(data: &[u8]) -> &'static str {
    if data.starts_with(PNG_START) {
        "image/png"
    } else if data.starts_with(BMP_START) {
        "image/bmp"
    } else if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        "image/jpeg"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> CameraConfig {
        CameraConfig { host: "192.168.1.50".into(), ..Default::default() }
    }

    #[test]
    fn url_normalises_a_missing_slash() {
        let c =
            CameraConfig { host: "192.168.1.50".into(), port: 8080, path: "snapshot.jpg".into(), ..Default::default() };
        assert_eq!(c.url(), "http://192.168.1.50:8080/snapshot.jpg");
    }

    #[test]
    fn raw_endpoint_is_host_and_port() {
        let c = CameraConfig { transport: "raw".into(), host: "192.168.1.50".into(), port: 9000, ..Default::default() };
        assert_eq!(c.endpoint(), "tcp://192.168.1.50:9000");
    }

    #[test]
    fn a_jpeg_passes() {
        let mut data = b"\xff\xd8".to_vec();
        data.extend(std::iter::repeat_n(b'x', 100));
        data.extend_from_slice(b"\xff\xd9");
        assert_eq!(validate(data.clone(), &cfg()).unwrap(), data);
    }

    #[test]
    fn an_html_error_page_is_rejected_with_its_content() {
        let err = validate(b"<html><body>401 Unauthorized</body></html>".to_vec(), &cfg()).unwrap_err().to_string();
        assert!(err.contains("不是 JPEG"), "{err}");
        assert!(err.contains("401"), "{err}");
    }

    #[test]
    fn empty_response() {
        assert!(validate(vec![], &cfg()).is_err());
    }

    #[test]
    fn oversized_response() {
        let mut data = b"\xff\xd8".to_vec();
        data.resize(13 * 1024 * 1024, b'x');
        assert!(validate(data, &cfg()).unwrap_err().to_string().contains("MB"));
    }

    #[test]
    fn no_host_configured() {
        assert!(capture(&CameraConfig::default()).unwrap_err().to_string().contains("尚未設定"));
    }

    #[test]
    fn unknown_transport() {
        let c = CameraConfig { host: "10.0.0.1".into(), transport: "rtsp".into(), ..Default::default() };
        assert!(capture(&c).is_err());
    }

    #[test]
    fn unreachable_host_names_the_address() {
        for transport in ["http", "raw"] {
            let c = CameraConfig {
                host: "127.0.0.1".into(),
                port: 1,
                timeout: 1.0,
                transport: transport.into(),
                ..Default::default()
            };
            let err = capture(&c).unwrap_err().to_string();
            assert!(err.contains("127.0.0.1:1"), "{transport}: {err}");
        }
    }

    #[test]
    fn raw_socket_reads_one_frame() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut trigger = [0u8; 6];
            sock.read_exact(&mut trigger).unwrap();
            assert_eq!(&trigger, b"SNAP\r\n");
            sock.write_all(b"\xff\xd8hello\xff\xd9").unwrap();
            // Keep the socket open: the reader must stop at the end marker.
            std::thread::sleep(Duration::from_millis(500));
        });
        let c = CameraConfig {
            transport: "raw".into(),
            host: "127.0.0.1".into(),
            port,
            trigger: "SNAP\\r\\n".into(),
            timeout: 3.0,
            ..Default::default()
        };
        let frame = capture(&c).unwrap();
        assert_eq!(frame.data, b"\xff\xd8hello\xff\xd9");
        server.join().unwrap();
    }

    #[test]
    fn folder_takes_the_newest_and_refuses_stale() {
        let dir = std::env::temp_dir().join(format!("urdwms-cam-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.jpg"), b"\xff\xd8old\xff\xd9").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(dir.join("b.png"), b"\x89PNG\r\n\x1a\nnew").unwrap();
        std::fs::write(dir.join("notes.txt"), b"ignored").unwrap();
        let c = CameraConfig { transport: "folder".into(), folder: dir.to_string_lossy().into(), ..Default::default() };
        let frame = capture(&c).unwrap();
        assert!(frame.data.starts_with(PNG_START));
        assert_eq!(frame.source.unwrap().name, "b.png");

        let old = SystemTime::now() - Duration::from_secs(3600);
        for name in ["a.jpg", "b.png"] {
            std::fs::File::options().write(true).open(dir.join(name)).unwrap().set_modified(old).unwrap();
        }
        assert!(matches!(capture(&c), Err(CameraError::Stale { .. })));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
