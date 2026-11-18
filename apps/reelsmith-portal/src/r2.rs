//! Cloudflare R2 request signing (S3 SigV4). This builds presigned URLs. The live
//! CreateMultipartUpload / CompleteMultipartUpload calls are deferred: they need network
//! credentials and a bucket this build does not contact. See `docs/portal/DESIGN.md`.

use crate::error::Error;
use crate::ids::hex;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug)]
pub struct R2Config {
    pub account_id: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: String,
}

impl R2Config {
    pub fn from_env() -> Result<Self, Error> {
        let account_id = required("R2_ACCOUNT_ID")?;
        let bucket = required("R2_BUCKET")?;
        let access_key_id = required("R2_ACCESS_KEY_ID")?;
        let secret_access_key = required("R2_SECRET_ACCESS_KEY")?;
        if account_id.contains('/') || bucket.contains('/') || access_key_id.is_empty() {
            return Err(Error::invalid("R2 account, bucket and access key must be set and must not contain a slash"));
        }
        Ok(Self { account_id, bucket, access_key_id, secret_access_key })
    }

    pub fn endpoint(&self) -> String {
        format!("https://{}.r2.cloudflarestorage.com", self.account_id)
    }
}

fn required(name: &str) -> Result<String, Error> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(Error::invalid(format!("{name} is not set"))),
    }
}

#[derive(Clone, Debug)]
pub struct SignedRequest {
    pub method: String,
    pub uri: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub payload_hash: String,
    pub region: String,
    pub service: String,
    pub amz_date: String,
}

pub fn payload_hash(body: &[u8]) -> String {
    hex(&Sha256::digest(body))
}

pub fn canonical_request(signed: &SignedRequest) -> String {
    let mut headers = signed.headers.clone();
    headers.sort_by(|a, b| a.0.cmp(&b.0));
    let canonical_headers = headers.iter().map(|(name, value)| format!("{}:{}\n", name.trim().to_ascii_lowercase(), value.trim())).collect::<String>();
    let signed_headers = headers.iter().map(|(name, _)| name.trim().to_ascii_lowercase()).collect::<Vec<_>>().join(";");
    let mut query = signed.query.clone();
    query.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let canonical_query = query.iter().map(|(key, value)| format!("{}={}", encode(key), encode(value))).collect::<Vec<_>>().join("&");
    format!("{}\n{}\n{}\n{}\n{}\n{}", signed.method, signed.uri, canonical_query, canonical_headers, signed_headers, signed.payload_hash)
}

pub fn sign(secret: &str, signed: &SignedRequest) -> Result<String, Error> {
    let canonical = canonical_request(signed);
    let date = signed.amz_date.get(..8).ok_or_else(|| Error::invalid("amz date is short"))?;
    let scope = format!("{date}/{}/{}/aws4_request", signed.region, signed.service);
    let string_to_sign = format!("AWS4-HMAC-SHA256\n{}\n{scope}\n{}", signed.amz_date, payload_hash(canonical.as_bytes()));
    let key = signing_key(secret, date, &signed.region, &signed.service)?;
    Ok(hex(&hmac_sha256(&key, string_to_sign.as_bytes())?))
}

pub fn presign_put(config: &R2Config, key: &str, amz_date: &str, expires: u64) -> Result<String, Error> {
    if expires == 0 || expires > 604_800 {
        return Err(Error::invalid("presign expiry must be between 1 second and 7 days"));
    }
    crate::storage::validate_key(key)?;
    let date = amz_date.get(..8).ok_or_else(|| Error::invalid("amz date is short"))?;
    let credential = format!("{}/{date}/auto/s3/aws4_request", config.access_key_id);
    let uri = format!("/{}/{}", config.bucket, key);
    let host = format!("{}.r2.cloudflarestorage.com", config.account_id);
    let query = vec![
        ("X-Amz-Algorithm".to_string(), "AWS4-HMAC-SHA256".to_string()),
        ("X-Amz-Credential".to_string(), credential),
        ("X-Amz-Date".to_string(), amz_date.to_string()),
        ("X-Amz-Expires".to_string(), expires.to_string()),
        ("X-Amz-SignedHeaders".to_string(), "host".to_string()),
    ];
    let request = SignedRequest {
        method: "PUT".to_string(),
        uri,
        query: query.clone(),
        headers: vec![("host".to_string(), host.clone())],
        payload_hash: "UNSIGNED-PAYLOAD".to_string(),
        region: "auto".to_string(),
        service: "s3".to_string(),
        amz_date: amz_date.to_string(),
    };
    let signature = sign(&config.secret_access_key, &request)?;
    let mut query = query;
    query.push(("X-Amz-Signature".to_string(), signature));
    let q = query.iter().map(|(k, v)| format!("{}={}", encode(k), encode(v))).collect::<Vec<_>>().join("&");
    let url = format!("https://{host}{}?{q}", request.uri);
    if url.contains(&config.secret_access_key) {
        return Err(Error::invalid("refusing to build a URL that contains the secret"));
    }
    Ok(url)
}

/// How many R2 parts a byte count becomes. R2's minimum part is 5 MiB except the last.
pub fn part_count(total_size: u64, part_size: u64) -> Result<u32, Error> {
    if part_size < 5 * 1024 * 1024 {
        return Err(Error::invalid("an R2 part must be at least 5 MiB"));
    }
    if total_size == 0 {
        return Err(Error::invalid("upload size is empty"));
    }
    let count = total_size.div_ceil(part_size);
    u32::try_from(count).map_err(|_| Error::invalid("too many parts"))
}

fn signing_key(secret: &str, date: &str, region: &str, service: &str) -> Result<Vec<u8>, Error> {
    let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date.as_bytes())?;
    let k_region = hmac_sha256(&k_date, region.as_bytes())?;
    let k_service = hmac_sha256(&k_region, service.as_bytes())?;
    hmac_sha256(&k_service, b"aws4_request")
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Result<Vec<u8>, Error> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| Error::invalid("hmac rejected the key"))?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
