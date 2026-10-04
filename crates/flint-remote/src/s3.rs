use std::fmt;
use std::time::Duration;

use object_store::aws::{AmazonS3, AmazonS3Builder, S3ConditionalPut};
use object_store::{ClientOptions, RetryConfig};
use url::{Host, Url};

use crate::{Error, ObjectStoreRemote, Result};

/// Where one store lives on S3 or an S3-compatible server.
#[derive(Clone)]
pub struct S3Config {
    pub bucket: String,
    /// The folder within the bucket for this store; one store = one bucket + prefix.
    pub prefix: String,
    pub region: String,
    /// `None` for AWS. Otherwise the server's URL, which must be `https` unless it is localhost.
    pub endpoint: Option<String>,
    pub access_key_id: String,
    pub secret_access_key: String,
}

impl fmt::Debug for S3Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3Config")
            .field("bucket", &self.bucket)
            .field("prefix", &self.prefix)
            .field("region", &self.region)
            .field("endpoint", &self.endpoint)
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &"<redacted>")
            .finish()
    }
}

impl ObjectStoreRemote<AmazonS3> {
    /// Connects to S3 or a compatible server, with conditional writes on and an `https`-only endpoint
    /// (plain `http` is accepted for localhost, where the emulators live). Makes no request; problems
    /// with credentials or the bucket show up on first use.
    pub fn s3(config: &S3Config) -> Result<Self> {
        let allow_http = match &config.endpoint {
            Some(endpoint) => check_endpoint(endpoint)?,
            None => false,
        };
        if config.bucket.is_empty() {
            return Err(Error::InvalidConfig("bucket name is empty".into()));
        }
        let mut builder = AmazonS3Builder::new()
            .with_bucket_name(&config.bucket)
            .with_region(&config.region)
            .with_access_key_id(&config.access_key_id)
            .with_secret_access_key(&config.secret_access_key)
            // `If-Match` / `If-None-Match` on PUT: how a concurrent change is detected
            .with_conditional_put(S3ConditionalPut::ETagMatch)
            // one options value: calling `with_allow_http` separately would be overwritten by it
            .with_client_options(
                ClientOptions::new()
                    .with_allow_http(allow_http)
                    .with_connect_timeout(Duration::from_secs(10))
                    .with_timeout(Duration::from_secs(120)),
            )
            .with_retry(RetryConfig {
                max_retries: 3,
                retry_timeout: Duration::from_secs(60),
                ..Default::default()
            });
        if let Some(endpoint) = &config.endpoint {
            // self-hosted servers address buckets by path, not by subdomain
            builder = builder
                .with_endpoint(endpoint)
                .with_virtual_hosted_style_request(false);
        }
        let store = builder
            .build()
            .map_err(|e| Error::InvalidConfig(e.to_string()))?;
        Ok(Self::new(store, &config.prefix))
    }
}

/// Accepts `https` anywhere and `http` only for localhost. Returns whether plain http is in use.
pub(crate) fn check_endpoint(endpoint: &str) -> Result<bool> {
    let url = Url::parse(endpoint)
        .map_err(|e| Error::InvalidConfig(format!("endpoint {endpoint:?}: {e}")))?;
    match url.scheme() {
        "https" => Ok(false),
        "http" => {
            let is_local = match url.host() {
                Some(Host::Domain(domain)) => {
                    domain.eq_ignore_ascii_case("localhost")
                        || domain.to_ascii_lowercase().ends_with(".localhost")
                }
                Some(Host::Ipv4(ip)) => ip.is_loopback(),
                Some(Host::Ipv6(ip)) => ip.is_loopback(),
                None => false,
            };
            if is_local {
                Ok(true)
            } else {
                Err(Error::InsecureEndpoint(endpoint.to_owned()))
            }
        }
        other => Err(Error::InvalidConfig(format!(
            "endpoint {endpoint:?} uses unsupported scheme {other:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(endpoint: Option<&str>) -> S3Config {
        S3Config {
            bucket: "bucket".into(),
            prefix: "store".into(),
            region: "us-east-1".into(),
            endpoint: endpoint.map(str::to_owned),
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: "secret".into(),
        }
    }

    #[test]
    fn https_is_accepted_anywhere() {
        for endpoint in [
            "https://s3.example.com",
            "https://s3.eu-west-1.amazonaws.com",
            "https://192.168.1.10:9000",
            "HTTPS://Example.com/path",
        ] {
            assert!(!check_endpoint(endpoint).unwrap(), "{endpoint}");
            assert!(
                ObjectStoreRemote::s3(&config(Some(endpoint))).is_ok(),
                "{endpoint}"
            );
        }
    }

    #[test]
    fn plain_http_is_accepted_only_for_localhost() {
        for endpoint in [
            "http://localhost:9000",
            "http://LOCALHOST",
            "http://127.0.0.1:8014",
            "http://127.5.5.5",
            "http://[::1]:9000",
            "http://minio.localhost:9000",
        ] {
            assert!(check_endpoint(endpoint).unwrap(), "{endpoint}");
            assert!(
                ObjectStoreRemote::s3(&config(Some(endpoint))).is_ok(),
                "{endpoint}"
            );
        }
    }

    #[test]
    fn plain_http_to_anything_else_is_refused() {
        for endpoint in [
            "http://s3.example.com",
            "http://192.168.1.10:9000",
            "http://10.0.0.5",
            "http://pi.local:9000",
            "http://localhost.evil.com",
            "http://127.0.0.1.evil.com",
            "http://example.com/localhost",
            "http://user@evil.com@localhost.evil.com",
            "http://0.0.0.0:9000",
        ] {
            assert!(
                matches!(check_endpoint(endpoint), Err(Error::InsecureEndpoint(_))),
                "{endpoint}"
            );
            assert!(
                matches!(
                    ObjectStoreRemote::s3(&config(Some(endpoint))),
                    Err(Error::InsecureEndpoint(_))
                ),
                "{endpoint}"
            );
        }
    }

    #[test]
    fn nonsense_endpoints_are_a_config_error() {
        for endpoint in [
            "",
            "not a url",
            "ftp://example.com",
            "s3://bucket",
            "//example.com",
            "file:///etc/passwd",
        ] {
            assert!(
                matches!(check_endpoint(endpoint), Err(Error::InvalidConfig(_))),
                "{endpoint:?}"
            );
        }
    }

    #[test]
    fn aws_needs_no_endpoint_and_an_empty_bucket_is_refused() {
        assert!(ObjectStoreRemote::s3(&config(None)).is_ok());
        let mut empty = config(None);
        empty.bucket.clear();
        assert!(matches!(
            ObjectStoreRemote::s3(&empty),
            Err(Error::InvalidConfig(_))
        ));
    }

    #[test]
    fn debug_output_hides_the_secret() {
        let shown = format!("{:?}", config(Some("https://s3.example.com")));
        assert!(!shown.contains("secret\""), "{shown}");
        assert!(shown.contains("<redacted>"));
        assert!(shown.contains("AKIDEXAMPLE"));
    }
}
