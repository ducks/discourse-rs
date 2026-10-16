//! The S3 test server: MinIO, started by the test that needs it on a port
//! of its own, with a fresh data directory, and stopped when dropped.
//! Cases and tests name it as `http://minio.localhost:9122` (what the
//! reference records with, scripts/s3-test-server); Config's
//! object_storage_endpoints sends the port's requests to the real port.
//!
//! MinIO comes from shell.nix (`minio` on PATH) or DISCOURSE_RS_MINIO.
//! Without it the tests that need it are skipped, saying so.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The endpoint cases configure (`s3_endpoint`).
pub const ENDPOINT: &str = "http://minio.localhost:9122";
/// MINIO_DOMAIN: buckets as hosts under it (virtual-hosted requests).
pub const DOMAIN: &str = "minio.localhost";
pub const REGION: &str = "us-east-1";
pub const ACCESS_KEY_ID: &str = "discourse-rs-test";
pub const SECRET_ACCESS_KEY: &str = "discourse-rs-test-secret";

/// A running MinIO.
pub struct S3Server {
    child: Child,
    dir: PathBuf,
    /// Where it listens: `http://minio.localhost:<port>`.
    pub endpoint: String,
}

/// The MinIO binary, if there is one.
fn minio() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("DISCOURSE_RS_MINIO") {
        return Some(PathBuf::from(path));
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join("minio"))
            .find(|path| path.is_file())
    })
}

impl S3Server {
    /// Whether MinIO can be started here, or why not.
    pub fn available() -> Result<(), String> {
        minio()
            .map(|_| ())
            .ok_or_else(|| "no MinIO: run inside nix-shell, or set DISCOURSE_RS_MINIO".into())
    }

    /// MinIO started and answering, or why not (no binary: the caller
    /// skips).
    pub fn start() -> Result<S3Server, String> {
        let Some(binary) = minio() else {
            return Err("no MinIO: run inside nix-shell, or set DISCOURSE_RS_MINIO".into());
        };
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map_err(|e| format!("no free port: {e}"))?
            .port();
        let dir =
            std::env::temp_dir().join(format!("discourse-rs-minio-{}-{port}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let child = Command::new(&binary)
            .args(["server", "--quiet", "--address"])
            .arg(format!(":{port}"))
            .arg(&dir)
            .env("MINIO_ROOT_USER", ACCESS_KEY_ID)
            .env("MINIO_ROOT_PASSWORD", SECRET_ACCESS_KEY)
            .env("MINIO_DOMAIN", DOMAIN)
            .env("MINIO_BROWSER", "off")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("starting {}: {e}", binary.display()))?;
        let server = S3Server {
            child,
            dir,
            endpoint: format!("http://{DOMAIN}:{port}"),
        };
        // Up once it answers S3 requests: before that (its health checks
        // pass sooner) it says XMinioServerNotInitialized.
        let root = format!("http://127.0.0.1:{port}/");
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(30) {
            let answer = std::process::Command::new("curl")
                .args(["-s", &root])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default();
            if answer.contains("<Error>") && !answer.contains("NotInitialized") {
                return Ok(server);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Err("MinIO did not come up in 30 seconds".into())
    }

    /// The (configured, real) pair for Config::object_storage_endpoints.
    pub fn endpoint_mapping(&self) -> (String, String) {
        (ENDPOINT.to_string(), self.endpoint.clone())
    }

    /// A bucket on the server, through the port's own S3 backend.
    pub fn bucket(&self, name: &str) -> discourse_rs::object_storage::Backend {
        let config = discourse_rs::object_storage::s3::S3Config {
            bucket: name.to_string(),
            region: REGION.to_string(),
            endpoint: Some(self.endpoint.clone()),
            access_key_id: ACCESS_KEY_ID.to_string(),
            secret_access_key: SECRET_ACCESS_KEY.to_string(),
            session_token: None,
            force_path_style: false,
            dualstack: false,
            accelerate: false,
        };
        discourse_rs::object_storage::Backend::S3(
            discourse_rs::object_storage::s3::S3::new(&config).expect("an S3 bucket"),
        )
    }

    /// The site settings that put a site's uploads in `bucket` here.
    pub fn upload_settings(bucket: &str) -> Vec<(&'static str, String)> {
        vec![
            ("s3_upload_bucket", bucket.to_string()),
            ("s3_access_key_id", ACCESS_KEY_ID.to_string()),
            ("s3_secret_access_key", SECRET_ACCESS_KEY.to_string()),
            ("s3_region", REGION.to_string()),
            ("s3_endpoint", ENDPOINT.to_string()),
            ("enable_s3_uploads", "t".to_string()),
        ]
    }
}

impl Drop for S3Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
