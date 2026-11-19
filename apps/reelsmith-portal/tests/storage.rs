//! Local resumable uploads, and the rule that a finished source cannot be deleted.

use reelsmith_portal::r2::{self, SignedRequest, canonical_request, sign};
use reelsmith_portal::storage::LocalStore;

#[test]
fn parts_can_arrive_out_of_order_and_the_source_is_kept() {
    let dir = tempfile::tempdir().expect("temp");
    let store = LocalStore::open(dir.path()).expect("store");
    let upload = store.create_upload("c/harbor/aroll/take/hook.bin", 11, "aroll", 1024).expect("create");
    store.write_part(&upload.id, 2, b"world").expect("part 2");
    store.write_part(&upload.id, 1, b"hello ").expect("part 1");
    let status = store.status(&upload.id).expect("status");
    assert_eq!(status.parts, vec![1, 2]);
    let done = store.complete(&upload.id).expect("complete");
    assert_eq!(store.read(&done.key).expect("bytes"), b"hello world");
    assert!(done.sha256.is_some());
    let again = store.complete(&upload.id).expect("second complete is the same object");
    assert_eq!(again.sha256, done.sha256);
    assert!(store.delete_source(&done.key).is_err());
    assert!(store.create_upload(&done.key, 4, "aroll", 1024).is_err(), "a source key cannot be replaced");
}

#[test]
fn hostile_keys_and_incomplete_uploads_are_rejected() {
    let dir = tempfile::tempdir().expect("temp");
    let store = LocalStore::open(dir.path()).expect("store");
    assert!(store.create_upload("../secret", 4, "aroll", 1024).is_err());
    assert!(store.create_upload("/etc/passwd", 4, "aroll", 1024).is_err());
    assert!(store.create_upload("c/ok/aroll/file", 0, "aroll", 1024).is_err());
    let upload = store.create_upload("c/ok/aroll/file.bin", 4, "aroll", 1024).expect("create");
    store.write_part(&upload.id, 1, b"ab").expect("short");
    assert!(store.complete(&upload.id).is_err(), "declared size does not match");
    assert!(store.read("c/ok/aroll/file.bin").is_err(), "incomplete bytes are not a source");
}

#[test]
fn sigv4_matches_the_published_iam_example_and_hides_the_secret() {
    // AWS general reference, "Signature Calculations for the Authorization Header, Example 1".
    let request = SignedRequest {
        method: "GET".to_string(),
        uri: "/".to_string(),
        query: vec![("Action".to_string(), "ListUsers".to_string()), ("Version".to_string(), "2010-05-08".to_string())],
        headers: vec![
            ("content-type".to_string(), "application/x-www-form-urlencoded; charset=utf-8".to_string()),
            ("host".to_string(), "iam.amazonaws.com".to_string()),
            ("x-amz-date".to_string(), "20150830T123600Z".to_string()),
        ],
        payload_hash: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
        region: "us-east-1".to_string(),
        service: "iam".to_string(),
        amz_date: "20150830T123600Z".to_string(),
    };
    let canonical = canonical_request(&request);
    assert!(canonical.contains("content-type:application/x-www-form-urlencoded; charset=utf-8\n"));
    let signature = sign("wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY", &request).expect("sign");
    assert_eq!(signature, "5d672d79c15b13162d9279b0855cfba6789a8edb4c82c400e06b5924a6f2b5d7");
    let config = r2::R2Config {
        account_id: "account".to_string(),
        bucket: "bucket".to_string(),
        access_key_id: "AKIDEXAMPLE".to_string(),
        secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".to_string(),
    };
    let url = r2::presign_put(&config, "c/harbor/aroll/hook.bin", "20150830T123600Z", 3600).expect("presign");
    assert!(url.contains("X-Amz-Signature="));
    assert!(!url.contains(&config.secret_access_key));
    assert_eq!(r2::part_count(10 * 1024 * 1024, 5 * 1024 * 1024).expect("parts"), 2);
    assert!(r2::part_count(10, 1024).is_err());
}
