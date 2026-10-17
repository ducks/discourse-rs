//! The S3 API backend, for any service that speaks it: AWS, Cloudflare
//! R2, Backblaze B2, MinIO, Wasabi, DigitalOcean Spaces, Garage. Requests
//! are signed (AWS Signature V4, query-string form) by rusty-s3 and sent
//! with the port's reqwest client; responses are read here.
//!
//! Where requests go follows aws-sdk-s3, which Rails uses: a configured
//! endpoint (`s3_endpoint`) takes them, with the bucket in the host
//! (virtual-hosted) unless the endpoint is an IP address or path-style
//! addressing is asked for (`s3_force_path_style`); without one, AWS's
//! regional endpoint for `s3_region`.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::Path;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use chrono::NaiveDateTime;
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use url::Url;

use super::{LifecycleRule, ObjectMeta, Part, PutOptions, StorageError};

/// S3Helper::FIFTEEN_MEGABYTES: from this size a file goes up in parts.
pub const MULTIPART_THRESHOLD: u64 = 15 * 1024 * 1024;

/// How long the requests the port signs for itself stay valid.
const REQUEST_EXPIRES: u64 = 3600;

/// What the S3 backend is configured with (S3Helper.s3_options and the
/// bucket).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Config {
    /// The bucket's name, without a folder.
    pub bucket: String,
    pub region: String,
    /// `s3_endpoint`: blank for AWS's own.
    pub endpoint: Option<String>,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
    /// Requests address the bucket in the path, whatever the endpoint.
    pub force_path_style: bool,
    /// AWS's dual-stack endpoint (`use_dualstack_endpoint`).
    pub dualstack: bool,
    /// AWS's transfer acceleration endpoint
    /// (`enable_s3_transfer_acceleration`).
    pub accelerate: bool,
}

/// A bucket on an S3 API service.
#[derive(Debug, Clone)]
pub struct S3 {
    bucket: Bucket,
    credentials: Credentials,
    region: String,
}

/// The HTTP client for every bucket: one connection pool, rustls.
fn client() -> &'static reqwest::Client {
    static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("an HTTP client")
    });
    &CLIENT
}

/// Buckets known to be there (S3Helper#s3_bucket checks once per helper;
/// the port once per process).
static KNOWN_BUCKETS: LazyLock<Mutex<std::collections::HashSet<String>>> =
    LazyLock::new(Default::default);

impl S3 {
    pub fn new(config: &S3Config) -> Result<S3, StorageError> {
        let (endpoint, style) = endpoint(config)?;
        let bucket = Bucket::new(
            endpoint,
            style,
            config.bucket.clone(),
            config.region.clone(),
        )
        .map_err(|e| StorageError::Transport(format!("invalid S3 endpoint: {e:?}")))?;
        let credentials = match &config.session_token {
            Some(token) => Credentials::new_with_token(
                config.access_key_id.clone(),
                config.secret_access_key.clone(),
                token.clone(),
            ),
            None => Credentials::new(
                config.access_key_id.clone(),
                config.secret_access_key.clone(),
            ),
        };
        Ok(S3 {
            bucket,
            credentials,
            region: config.region.clone(),
        })
    }

    pub fn bucket(&self) -> &str {
        self.bucket.name()
    }

    /// Where the bucket's requests go (`https://bucket.host/` or
    /// `http://host/bucket/`).
    pub fn base_url(&self) -> &Url {
        self.bucket.base_url()
    }

    fn object_url(&self, key: &str) -> Result<Url, StorageError> {
        self.bucket
            .object_url(key)
            .map_err(|e| StorageError::Transport(format!("invalid S3 key {key:?}: {e}")))
    }

    /// A request signed for `expires_in` seconds: `query` and `headers`
    /// (names lowercase) signed with it, the query in the order given, as
    /// aws-sdk lays a presigned URL out, then the signature's own.
    fn presign(
        &self,
        method: rusty_s3::Method,
        url: Url,
        query: &[(&str, &str)],
        headers: &[(String, String)],
        expires_in: u64,
    ) -> Url {
        let mut sorted_query: Vec<(&str, &str)> = query.to_vec();
        sorted_query.sort();
        let mut sorted_headers: Vec<(&str, &str)> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        sorted_headers.sort();
        let signed = rusty_s3::signing::sign(
            &jiff::Timestamp::now(),
            method,
            url.clone(),
            self.credentials.key(),
            self.credentials.secret(),
            self.credentials.token(),
            &self.region,
            expires_in,
            sorted_query.iter().copied(),
            sorted_headers.iter().copied(),
        );
        let signature: BTreeMap<String, String> = signed
            .query_pairs()
            .filter(|(k, _)| k.starts_with("X-Amz-"))
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        let mut out = url;
        let mut pairs: Vec<String> = query
            .iter()
            .map(|(k, v)| format!("{}={}", uri_escape(k), uri_escape(v)))
            .collect();
        for name in [
            "X-Amz-Algorithm",
            "X-Amz-Credential",
            "X-Amz-Date",
            "X-Amz-Expires",
            "X-Amz-Security-Token",
            "X-Amz-SignedHeaders",
            "X-Amz-Signature",
        ] {
            if let Some(value) = signature.get(name) {
                pairs.push(format!("{name}={}", uri_escape(value)));
            }
        }
        out.set_query(Some(&pairs.join("&")));
        out
    }

    /// A request sent: signed with `query` and `headers`, which go with it.
    async fn send(
        &self,
        method: rusty_s3::Method,
        url: Url,
        query: &[(&str, &str)],
        headers: &[(String, String)],
        body: Option<Vec<u8>>,
    ) -> Result<reqwest::Response, StorageError> {
        let signed = self.presign(method, url, query, headers, REQUEST_EXPIRES);
        let method = match method {
            rusty_s3::Method::Get => reqwest::Method::GET,
            rusty_s3::Method::Head => reqwest::Method::HEAD,
            rusty_s3::Method::Put => reqwest::Method::PUT,
            rusty_s3::Method::Post => reqwest::Method::POST,
            rusty_s3::Method::Delete => reqwest::Method::DELETE,
        };
        // aws-sdk's standard retries: three attempts at a server error or
        // a dropped connection, backing off in between.
        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut request = client().request(method.clone(), signed.as_str());
            for (name, value) in headers {
                request = request.header(name, value);
            }
            if let Some(body) = &body {
                request = request.body(body.clone());
            }
            let retry = attempt < 3;
            let response = match request.send().await {
                Ok(response) => response,
                Err(e) if retry && (e.is_connect() || e.is_timeout()) => {
                    tokio::time::sleep(Duration::from_millis(100 << attempt)).await;
                    continue;
                }
                Err(e) => return Err(StorageError::Transport(e.to_string())),
            };
            if response.status().is_success() {
                return Ok(response);
            }
            if retry && response.status().is_server_error() {
                tokio::time::sleep(Duration::from_millis(100 << attempt)).await;
                continue;
            }
            return Err(error_from(response).await);
        }
    }

    pub async fn ensure_bucket(&self) -> Result<(), StorageError> {
        let id = self.bucket.base_url().to_string();
        if KNOWN_BUCKETS.lock().expect("bucket cache").contains(&id) {
            return Ok(());
        }
        let head = self
            .send(
                rusty_s3::Method::Head,
                self.bucket.base_url().clone(),
                &[],
                &[],
                None,
            )
            .await;
        match head {
            Ok(_) => {}
            Err(StorageError::Service { status: 404, .. }) | Err(StorageError::NotFound) => {
                // aws-sdk names the region outside us-east-1.
                let body = (self.region != "us-east-1").then(|| {
                    format!(
                        "<CreateBucketConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
                         <LocationConstraint>{}</LocationConstraint></CreateBucketConfiguration>",
                        xml_escape(&self.region)
                    )
                    .into_bytes()
                });
                let created = self
                    .send(
                        rusty_s3::Method::Put,
                        self.bucket.base_url().clone(),
                        &[],
                        &[],
                        body,
                    )
                    .await;
                match created {
                    Ok(_) => {}
                    Err(e) if matches!(e.code(), "BucketAlreadyOwnedByYou") => {}
                    Err(e) => return Err(e),
                }
            }
            Err(e) => return Err(e),
        }
        KNOWN_BUCKETS.lock().expect("bucket cache").insert(id);
        Ok(())
    }

    pub async fn put(
        &self,
        key: &str,
        body: Vec<u8>,
        opts: &PutOptions,
    ) -> Result<String, StorageError> {
        // aws-sdk sends a body without a type as application/octet-stream.
        let typed;
        let opts = if opts.content_type.is_none() {
            typed = PutOptions {
                content_type: Some("application/octet-stream".into()),
                ..opts.clone()
            };
            &typed
        } else {
            opts
        };
        let url = self.object_url(key)?;
        let headers = put_headers(opts);
        let sent = self
            .send(
                rusty_s3::Method::Put,
                url.clone(),
                &[],
                &headers,
                Some(body.clone()),
            )
            .await;
        let response = match sent {
            // S3Helper#upload: a Content-Disposition too large for the
            // metadata goes, the rest stays.
            Err(e) if e.code() == "MetadataTooLarge" && opts.content_disposition.is_some() => {
                let opts = PutOptions {
                    content_disposition: None,
                    ..opts.clone()
                };
                self.send(
                    rusty_s3::Method::Put,
                    url,
                    &[],
                    &put_headers(&opts),
                    Some(body),
                )
                .await?
            }
            other => other?,
        };
        Ok(etag_header(&response))
    }

    pub async fn put_file(
        &self,
        key: &str,
        path: &Path,
        opts: &PutOptions,
    ) -> Result<String, StorageError> {
        let size = tokio::fs::metadata(path).await?.len();
        if size < MULTIPART_THRESHOLD {
            return self.put(key, tokio::fs::read(path).await?, opts).await;
        }
        use tokio::io::AsyncReadExt;
        let upload_id = self.create_multipart(key, opts).await?;
        let uploaded = async {
            let mut file = tokio::fs::File::open(path).await?;
            let mut parts = Vec::new();
            let mut number: u16 = 0;
            loop {
                let mut chunk = Vec::with_capacity(MULTIPART_THRESHOLD as usize);
                (&mut file)
                    .take(MULTIPART_THRESHOLD)
                    .read_to_end(&mut chunk)
                    .await?;
                if chunk.is_empty() {
                    break;
                }
                number = number
                    .checked_add(1)
                    .ok_or_else(|| StorageError::Transport("too many parts".into()))?;
                let mut url = self.object_url(key)?;
                url.query_pairs_mut().clear();
                let n = number.to_string();
                let response = self
                    .send(
                        rusty_s3::Method::Put,
                        url,
                        &[("partNumber", &n), ("uploadId", &upload_id)],
                        &[],
                        Some(chunk),
                    )
                    .await?;
                parts.push(Part {
                    number,
                    etag: etag_header(&response),
                });
            }
            self.complete_multipart(key, &upload_id, &parts).await
        };
        match uploaded.await {
            Ok(()) => {}
            Err(e) => {
                let _ = self.abort_multipart(key, &upload_id).await;
                return Err(e);
            }
        }
        Ok(self
            .head(key)
            .await?
            .map(|meta| meta.etag)
            .unwrap_or_default())
    }

    pub async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        let url = self.object_url(key)?;
        match self.send(rusty_s3::Method::Get, url, &[], &[], None).await {
            Ok(response) => Ok(Some(
                response
                    .bytes()
                    .await
                    .map_err(|e| StorageError::Transport(e.to_string()))?
                    .to_vec(),
            )),
            Err(StorageError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn download(&self, key: &str, destination: &Path) -> Result<(), StorageError> {
        use tokio::io::AsyncWriteExt;
        let url = self.object_url(key)?;
        let mut response = self
            .send(rusty_s3::Method::Get, url, &[], &[], None)
            .await?;
        let mut file = tokio::fs::File::create(destination).await?;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| StorageError::Transport(e.to_string()))?
        {
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        Ok(())
    }

    pub async fn head(&self, key: &str) -> Result<Option<ObjectMeta>, StorageError> {
        let url = self.object_url(key)?;
        let response = match self.send(rusty_s3::Method::Head, url, &[], &[], None).await {
            Ok(response) => response,
            Err(StorageError::NotFound) => return Ok(None),
            Err(e) => return Err(e),
        };
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let size = header("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let last_modified = header("last-modified")
            .and_then(|v| chrono::DateTime::parse_from_rfc2822(&v).ok())
            .map(|t| t.naive_utc())
            .unwrap_or_default();
        let metadata = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                let name = name.as_str().strip_prefix("x-amz-meta-")?;
                Some((name.to_string(), value.to_str().ok()?.to_string()))
            })
            .collect();
        Ok(Some(ObjectMeta {
            key: key.to_string(),
            size,
            etag: etag_header(&response),
            last_modified,
            metadata,
            content_type: header("content-type"),
            cache_control: header("cache-control"),
            content_disposition: header("content-disposition"),
        }))
    }

    pub async fn delete(&self, key: &str) -> Result<(), StorageError> {
        let url = self.object_url(key)?;
        match self
            .send(rusty_s3::Method::Delete, url, &[], &[], None)
            .await
        {
            Ok(_) | Err(StorageError::NotFound) => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub async fn copy(
        &self,
        source: &str,
        destination: &str,
        replace: Option<&PutOptions>,
    ) -> Result<String, StorageError> {
        let url = self.object_url(destination)?;
        let mut headers = vec![(
            "x-amz-copy-source".to_string(),
            format!("{}/{}", self.bucket.name(), path_escape(source)),
        )];
        if let Some(opts) = replace {
            headers.extend(put_headers(opts));
            headers.push((
                "x-amz-metadata-directive".to_string(),
                "REPLACE".to_string(),
            ));
            if opts.tagging.is_some() {
                headers.push(("x-amz-tagging-directive".to_string(), "REPLACE".to_string()));
            }
        }
        let response = self
            .send(rusty_s3::Method::Put, url, &[], &headers, Some(Vec::new()))
            .await?;
        let body = response
            .text()
            .await
            .map_err(|e| StorageError::Transport(e.to_string()))?;
        // A copy can fail after its 200 (the error in the body).
        if let Some(code) = xml_text(&body, "Code") {
            return Err(StorageError::Service {
                status: 200,
                message: xml_text(&body, "Message").unwrap_or_default(),
                code,
            });
        }
        Ok(xml_text(&body, "ETag")
            .map(|e| unquote(&xml_unescape(&e)))
            .unwrap_or_default())
    }

    pub async fn list(&self, prefix: &str) -> Result<Vec<ObjectMeta>, StorageError> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut action = self.bucket.list_objects_v2(Some(&self.credentials));
            if !prefix.is_empty() {
                action.with_prefix(prefix);
            }
            if let Some(t) = &token {
                action.with_continuation_token(t.clone());
            }
            let url = action.sign(Duration::from_secs(REQUEST_EXPIRES));
            let response = client()
                .get(url.as_str())
                .send()
                .await
                .map_err(|e| StorageError::Transport(e.to_string()))?;
            if !response.status().is_success() {
                return Err(error_from(response).await);
            }
            let body = response
                .bytes()
                .await
                .map_err(|e| StorageError::Transport(e.to_string()))?;
            let parsed = rusty_s3::actions::ListObjectsV2::parse_response(&body)
                .map_err(|e| StorageError::Transport(format!("ListObjectsV2: {e}")))?;
            for item in parsed.contents {
                out.push(ObjectMeta {
                    // encoding-type=url
                    key: percent_decode(&item.key),
                    size: item.size,
                    etag: unquote(&item.etag),
                    last_modified: parse_iso8601(&item.last_modified),
                    metadata: BTreeMap::new(),
                    content_type: None,
                    cache_control: None,
                    content_disposition: None,
                });
            }
            match parsed.next_continuation_token {
                Some(next) if !next.is_empty() => token = Some(next),
                _ => break,
            }
        }
        Ok(out)
    }

    pub fn presigned_get(
        &self,
        key: &str,
        expires_in: u64,
        response_content_disposition: Option<&str>,
    ) -> Result<String, StorageError> {
        let url = self.object_url(key)?;
        let mut query: Vec<(&str, &str)> = Vec::new();
        if let Some(disposition) = response_content_disposition {
            query.push(("response-content-disposition", disposition));
        }
        query.push(("x-id", "GetObject"));
        Ok(self
            .presign(rusty_s3::Method::Get, url, &query, &[], expires_in)
            .to_string())
    }

    pub fn presigned_put(
        &self,
        key: &str,
        expires_in: u64,
        opts: &PutOptions,
    ) -> Result<(String, BTreeMap<String, String>), StorageError> {
        let url = self.object_url(key)?;
        let headers = put_headers(opts);
        let signed = self.presign(
            rusty_s3::Method::Put,
            url,
            &[("x-id", "PutObject")],
            &headers,
            expires_in,
        );
        Ok((signed.to_string(), headers.into_iter().collect()))
    }

    pub async fn create_multipart(
        &self,
        key: &str,
        opts: &PutOptions,
    ) -> Result<String, StorageError> {
        let url = self.object_url(key)?;
        let response = self
            .send(
                rusty_s3::Method::Post,
                url,
                &[("uploads", "")],
                &put_headers(opts),
                Some(Vec::new()),
            )
            .await?;
        let body = response
            .text()
            .await
            .map_err(|e| StorageError::Transport(e.to_string()))?;
        xml_text(&body, "UploadId")
            .map(|id| xml_unescape(&id))
            .ok_or_else(|| StorageError::Transport("CreateMultipartUpload: no UploadId".into()))
    }

    pub fn presign_part(
        &self,
        key: &str,
        upload_id: &str,
        part_number: u16,
        expires_in: u64,
    ) -> Result<String, StorageError> {
        let url = self.object_url(key)?;
        let n = part_number.to_string();
        Ok(self
            .presign(
                rusty_s3::Method::Put,
                url,
                &[
                    ("partNumber", &n),
                    ("uploadId", upload_id),
                    ("x-id", "UploadPart"),
                ],
                &[],
                expires_in,
            )
            .to_string())
    }

    pub async fn list_parts(
        &self,
        key: &str,
        upload_id: &str,
        max_parts: u16,
    ) -> Result<Vec<Part>, StorageError> {
        let url = self.object_url(key)?;
        let max = max_parts.to_string();
        let response = self
            .send(
                rusty_s3::Method::Get,
                url,
                &[("max-parts", &max), ("uploadId", upload_id)],
                &[],
                None,
            )
            .await?;
        let body = response
            .bytes()
            .await
            .map_err(|e| StorageError::Transport(e.to_string()))?;
        let parsed = rusty_s3::actions::ListParts::parse_response(&body)
            .map_err(|e| StorageError::Transport(format!("ListParts: {e}")))?;
        Ok(parsed
            .parts
            .into_iter()
            .map(|p| Part {
                number: p.number,
                etag: unquote(&p.etag),
            })
            .collect())
    }

    pub async fn complete_multipart(
        &self,
        key: &str,
        upload_id: &str,
        parts: &[Part],
    ) -> Result<(), StorageError> {
        let url = self.object_url(key)?;
        let mut body = String::from("<CompleteMultipartUpload>");
        for part in parts {
            body.push_str(&format!(
                "<Part><PartNumber>{}</PartNumber><ETag>\"{}\"</ETag></Part>",
                part.number,
                xml_escape(part.etag.trim_matches('"'))
            ));
        }
        body.push_str("</CompleteMultipartUpload>");
        let response = self
            .send(
                rusty_s3::Method::Post,
                url,
                &[("uploadId", upload_id)],
                &[],
                Some(body.into_bytes()),
            )
            .await?;
        let text = response
            .text()
            .await
            .map_err(|e| StorageError::Transport(e.to_string()))?;
        // As a copy, a completion can fail in its 200's body.
        if text.contains("<Error>")
            && let Some(code) = xml_text(&text, "Code")
        {
            return Err(StorageError::Service {
                status: 200,
                message: xml_text(&text, "Message").unwrap_or_default(),
                code,
            });
        }
        Ok(())
    }

    pub async fn abort_multipart(&self, key: &str, upload_id: &str) -> Result<(), StorageError> {
        let url = self.object_url(key)?;
        self.send(
            rusty_s3::Method::Delete,
            url,
            &[("uploadId", upload_id)],
            &[],
            None,
        )
        .await?;
        Ok(())
    }

    /// The bucket's lifecycle configuration as S3 returns it, empty when
    /// it has none.
    async fn lifecycle_xml(&self) -> Result<String, StorageError> {
        let url = self.bucket.base_url().clone();
        match self
            .send(rusty_s3::Method::Get, url, &[("lifecycle", "")], &[], None)
            .await
        {
            Ok(response) => response
                .text()
                .await
                .map_err(|e| StorageError::Transport(e.to_string())),
            Err(e) if e.code() == "NoSuchLifecycleConfiguration" => Ok(String::new()),
            Err(e) => Err(e),
        }
    }

    /// The bucket's expiry rules: id, status, days and prefix.
    pub async fn lifecycle_rules(&self) -> Result<Vec<LifecycleRule>, StorageError> {
        let xml = self.lifecycle_xml().await?;
        Ok(xml_elements(&xml, "Rule")
            .into_iter()
            .map(|rule| LifecycleRule {
                id: xml_text(rule, "ID").map(|v| xml_unescape(&v)),
                status: xml_text(rule, "Status").unwrap_or_default(),
                days: xml_elements(rule, "Expiration")
                    .first()
                    .and_then(|e| xml_text(e, "Days"))
                    .and_then(|d| d.parse().ok()),
                prefix: xml_elements(rule, "Filter")
                    .first()
                    .and_then(|f| xml_text(f, "Prefix"))
                    .or_else(|| xml_text(rule, "Prefix"))
                    .map(|p| xml_unescape(&p)),
            })
            .collect())
    }

    /// S3Helper#update_lifecycle: the bucket's rules read, the one of this
    /// id (`_` and `-` alike) replaced by an expiry after `days` for the
    /// prefix, the rules written back.
    pub async fn update_lifecycle(
        &self,
        id: &str,
        days: i64,
        prefix: &str,
    ) -> Result<(), StorageError> {
        let url = self.bucket.base_url().clone();
        let existing = self.lifecycle_xml().await?;
        let normalized = id.replace('_', "-");
        let mut rules: Vec<String> = xml_elements(&existing, "Rule")
            .into_iter()
            .filter(|rule| {
                xml_text(rule, "ID")
                    .is_none_or(|rid| xml_unescape(&rid).replace('_', "-") != normalized)
            })
            .map(|rule| format!("<Rule>{rule}</Rule>"))
            .collect();
        rules.push(format!(
            "<Rule><ID>{}</ID><Status>Enabled</Status><Expiration><Days>{days}</Days></Expiration>\
             <Filter><Prefix>{}</Prefix></Filter></Rule>",
            xml_escape(id),
            xml_escape(prefix)
        ));
        let body = format!(
            "<LifecycleConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">{}</LifecycleConfiguration>",
            rules.concat()
        );
        // PutBucketLifecycleConfiguration wants the body's MD5.
        let md5 = {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(md5::compute(body.as_bytes()).0)
        };
        self.send(
            rusty_s3::Method::Put,
            url,
            &[("lifecycle", "")],
            &[("content-md5".to_string(), md5)],
            Some(body.into_bytes()),
        )
        .await?;
        Ok(())
    }
}

/// Where the requests go and how the bucket is addressed.
fn endpoint(config: &S3Config) -> Result<(Url, UrlStyle), StorageError> {
    let invalid = |e: url::ParseError| StorageError::Transport(format!("invalid s3_endpoint: {e}"));
    match config.endpoint.as_deref().filter(|e| !e.trim().is_empty()) {
        Some(endpoint) => {
            let url = Url::parse(endpoint).map_err(invalid)?;
            let ip = url
                .host_str()
                .map(|h| h.trim_start_matches('[').trim_end_matches(']'))
                .is_some_and(|h| h.parse::<IpAddr>().is_ok());
            // A dotted bucket name cannot be a host under TLS.
            let dotted = config.bucket.contains('.') && url.scheme() == "https";
            let style = if config.force_path_style || ip || dotted {
                UrlStyle::Path
            } else {
                UrlStyle::VirtualHost
            };
            Ok((url, style))
        }
        None => {
            let suffix = if config.region.starts_with("cn-") {
                "amazonaws.com.cn"
            } else {
                "amazonaws.com"
            };
            let host = if config.accelerate {
                format!("s3-accelerate.{suffix}")
            } else if config.dualstack {
                format!("s3.dualstack.{}.{suffix}", config.region)
            } else {
                format!("s3.{}.{suffix}", config.region)
            };
            let style = if config.force_path_style {
                UrlStyle::Path
            } else {
                UrlStyle::VirtualHost
            };
            Ok((
                Url::parse(&format!("https://{host}")).map_err(invalid)?,
                style,
            ))
        }
    }
}

/// The headers an object is written with, names lowercase.
fn put_headers(opts: &PutOptions) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    let mut add = |name: &str, value: &Option<String>| {
        if let Some(value) = value {
            headers.push((name.to_string(), value.clone()));
        }
    };
    add("cache-control", &opts.cache_control);
    add("content-disposition", &opts.content_disposition);
    add("content-type", &opts.content_type);
    add("x-amz-acl", &opts.acl);
    add("x-amz-tagging", &opts.tagging);
    for (name, value) in &opts.metadata {
        headers.push((format!("x-amz-meta-{}", name.to_lowercase()), value.clone()));
    }
    headers
}

/// A failed response as the error S3 names in its body (none for a HEAD:
/// the status names it, as aws-sdk does).
async fn error_from(response: reqwest::Response) -> StorageError {
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    let code = xml_text(&body, "Code").unwrap_or_else(|| {
        match status {
            301 => "MovedPermanently",
            400 => "BadRequest",
            403 => "Forbidden",
            404 => "NotFound",
            412 => "PreconditionFailed",
            _ => "Http",
        }
        .to_string()
    });
    if status == 404 && matches!(code.as_str(), "NoSuchKey" | "NotFound") {
        return StorageError::NotFound;
    }
    if code == "NoSuchUpload" {
        return StorageError::NoSuchUpload;
    }
    StorageError::Service {
        status,
        message: xml_text(&body, "Message")
            .map(|m| xml_unescape(&m))
            .unwrap_or_default(),
        code,
    }
}

fn etag_header(response: &reqwest::Response) -> String {
    response
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(unquote)
        .unwrap_or_default()
}

fn unquote(etag: &str) -> String {
    etag.replace('"', "")
}

/// The text of the first `<tag>` element (S3's XML has no attributes on
/// the ones read here).
fn xml_text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].to_string())
}

/// The insides of every `<tag>` element at any depth, in order.
fn xml_elements<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(&open) {
        let after = &rest[start + open.len()..];
        let Some(end) = after.find(&close) else {
            break;
        };
        out.push(&after[..end]);
        rest = &after[end + close.len()..];
    }
    out
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn xml_unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// `Aws::Sigv4::Signer.uri_escape`: everything but RFC 3986's unreserved
/// characters percent-encoded.
pub fn uri_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A key in a path: each segment escaped, the slashes kept.
fn path_escape(key: &str) -> String {
    key.split('/').map(uri_escape).collect::<Vec<_>>().join("/")
}

fn percent_decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s)
        .decode_utf8_lossy()
        .into_owned()
}

/// ListObjectsV2's `2026-10-16T13:46:58.123Z`.
fn parse_iso8601(s: &str) -> NaiveDateTime {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|t| t.naive_utc())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(endpoint: Option<&str>) -> S3Config {
        S3Config {
            bucket: "uploads".into(),
            region: "us-west-2".into(),
            endpoint: endpoint.map(str::to_string),
            access_key_id: "key".into(),
            secret_access_key: "secret".into(),
            session_token: None,
            force_path_style: false,
            dualstack: false,
            accelerate: false,
        }
    }

    #[test]
    fn requests_go_where_aws_sdk_sends_them() {
        let base = |c: &S3Config| S3::new(c).unwrap().base_url().to_string();
        // AWS's own, regional.
        assert_eq!(
            base(&config(None)),
            "https://uploads.s3.us-west-2.amazonaws.com/"
        );
        assert_eq!(
            base(&S3Config {
                dualstack: true,
                ..config(None)
            }),
            "https://uploads.s3.dualstack.us-west-2.amazonaws.com/"
        );
        // A provider's endpoint: virtual-hosted on a name.
        assert_eq!(
            base(&config(Some("https://nyc3.digitaloceanspaces.com"))),
            "https://uploads.nyc3.digitaloceanspaces.com/"
        );
        // Path style on an IP, or when asked.
        assert_eq!(
            base(&config(Some("http://127.0.0.1:9000"))),
            "http://127.0.0.1:9000/uploads/"
        );
        assert_eq!(
            base(&S3Config {
                force_path_style: true,
                ..config(Some("https://garage.example.com"))
            }),
            "https://garage.example.com/uploads/"
        );
    }

    #[test]
    fn presigned_urls_lay_out_as_aws_sdk_does() {
        let s3 = S3::new(&config(Some("http://127.0.0.1:9000"))).unwrap();
        let url = s3
            .presigned_get(
                "original/1X/a b.png",
                300,
                Some("attachment; filename=\"a b.png\""),
            )
            .unwrap();
        let (base, query) = url.split_once('?').unwrap();
        assert_eq!(base, "http://127.0.0.1:9000/uploads/original/1X/a%20b.png");
        let names: Vec<&str> = query
            .split('&')
            .map(|p| p.split_once('=').unwrap().0)
            .collect();
        assert_eq!(
            names,
            [
                "response-content-disposition",
                "x-id",
                "X-Amz-Algorithm",
                "X-Amz-Credential",
                "X-Amz-Date",
                "X-Amz-Expires",
                "X-Amz-SignedHeaders",
                "X-Amz-Signature"
            ]
        );
        assert!(query.starts_with(
            "response-content-disposition=attachment%3B%20filename%3D%22a%20b.png%22&x-id=GetObject&"
        ));
        assert!(query.contains("&X-Amz-Expires=300&X-Amz-SignedHeaders=host&"));
    }

    #[test]
    fn reads_s3_xml() {
        let body = "<Error><Code>NoSuchBucket</Code><Message>The specified bucket does not exist</Message></Error>";
        assert_eq!(xml_text(body, "Code").as_deref(), Some("NoSuchBucket"));
        let rules = "<LifecycleConfiguration><Rule><ID>a</ID></Rule><Rule><ID>b</ID></Rule></LifecycleConfiguration>";
        assert_eq!(xml_elements(rules, "Rule"), ["<ID>a</ID>", "<ID>b</ID>"]);
        assert_eq!(uri_escape("a b/c~"), "a%20b%2Fc~");
        assert_eq!(path_escape("temp/a b.png"), "temp/a%20b.png");
    }
}
