//! HTTP for the upload clients: requests with status codes as values,
//! `multipart/form-data` bodies like curl's `-F`, and retries with
//! exponential backoff for server errors and network failures (Publishing
//! §11.9, step 5).

// Only a computer has this (spec M14, decision T394).
#![allow(clippy::disallowed_methods)]

use std::time::Duration;

/// A response: status and body text.
#[derive(Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub body: String,
}

impl Response {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_str(&self.body).ok()
    }
}

/// A blocking HTTP client.
pub struct Http {
    agent: ureq::Agent,
    /// Waits between retries (shortened in tests).
    pub backoff: Duration,
}

impl Default for Http {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(300)))
            .build();
        Http {
            agent: ureq::Agent::new_with_config(config),
            backoff: std::env::var("WOWLUA_PUBLISH_BACKOFF_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .map(Duration::from_millis)
                .unwrap_or(Duration::from_secs(2)),
        }
    }
}

impl Http {
    /// One request.
    pub fn request(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, String)],
        body: Option<&[u8]>,
    ) -> Result<Response, String> {
        let mut req = ureq::http::Request::builder().method(method).uri(url);
        for (k, v) in headers {
            req = req.header(*k, v.as_str());
        }
        let result = match body {
            Some(b) => {
                let req = req.body(b.to_vec()).map_err(|e| e.to_string())?;
                self.agent.run(req)
            }
            None => {
                let req = req.body(()).map_err(|e| e.to_string())?;
                self.agent.run(req)
            }
        };
        let response = result.map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        let body = response
            .into_body()
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_string()
            .unwrap_or_default();
        Ok(Response { status, body })
    }

    /// A request retried up to three times on network errors and 5xx
    /// responses, waiting `backoff`, then twice and four times as long.
    pub fn retried(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, String)],
        body: Option<&[u8]>,
    ) -> Result<Response, String> {
        let mut wait = self.backoff;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let result = self.request(method, url, headers, body);
            let retry = match &result {
                Ok(r) => r.status >= 500,
                Err(_) => true,
            };
            if !retry || attempt >= 4 {
                return result;
            }
            std::thread::sleep(wait);
            wait *= 2;
        }
    }
}

/// A `multipart/form-data` body.
pub struct Multipart {
    boundary: String,
    body: Vec<u8>,
}

impl Default for Multipart {
    fn default() -> Self {
        let boundary = format!(
            "------------------------wowlua{:016x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0)
        );
        Multipart {
            boundary,
            body: Vec::new(),
        }
    }
}

impl Multipart {
    pub fn text(mut self, name: &str, value: &str) -> Self {
        self.body.extend_from_slice(
            format!(
                "--{}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n",
                self.boundary
            )
            .as_bytes(),
        );
        self.body.extend_from_slice(value.as_bytes());
        self.body.extend_from_slice(b"\r\n");
        self
    }

    pub fn file(mut self, name: &str, file_name: &str, content_type: &str, data: &[u8]) -> Self {
        self.body.extend_from_slice(
            format!(
                "--{}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{file_name}\"\r\nContent-Type: {content_type}\r\n\r\n",
                self.boundary
            )
            .as_bytes(),
        );
        self.body.extend_from_slice(data);
        self.body.extend_from_slice(b"\r\n");
        self
    }

    /// The content type header and the body.
    pub fn finish(mut self) -> (String, Vec<u8>) {
        self.body
            .extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());
        (
            format!("multipart/form-data; boundary={}", self.boundary),
            self.body,
        )
    }
}
