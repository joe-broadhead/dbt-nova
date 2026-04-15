use std::fs;

use hf_hub::{Cache, api::sync::ApiBuilder};
use tempfile::TempDir;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
#[ignore = "requires local socket bind for wiremock; run explicitly in environments that allow loopback bind"]
async fn sync_api_download_handles_relative_redirect_locations() {
    let server = MockServer::start().await;
    let model_path = "/test-model/resolve/main/onnx/model.onnx";
    let asset_path = "/cdn/test-model/onnx/model.onnx";

    Mock::given(method("GET"))
        .and(path(model_path))
        .respond_with(
            ResponseTemplate::new(302)
                .append_header("x-repo-commit", "abc123")
                .append_header("x-linked-etag", "etag-123")
                .append_header("Location", asset_path),
        )
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path(asset_path))
        .and(header("Range", "bytes=0-0"))
        .respond_with(
            ResponseTemplate::new(206)
                .append_header("Content-Range", "bytes 0-0/5")
                .set_body_bytes(vec![b'm']),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path(asset_path))
        .respond_with(ResponseTemplate::new(200).set_body_string("model"))
        .mount(&server)
        .await;

    let cache_dir = TempDir::new().expect("tempdir");
    let api = ApiBuilder::from_cache(Cache::new(cache_dir.path().to_path_buf()))
        .with_endpoint(server.uri())
        .with_progress(false)
        .build()
        .expect("build api");

    let downloaded = api
        .model("test-model".to_string())
        .download("onnx/model.onnx")
        .expect("download should succeed");

    assert_eq!(
        fs::read_to_string(&downloaded).expect("read downloaded file"),
        "model"
    );
    assert_eq!(
        fs::read_to_string(
            cache_dir
                .path()
                .join("models--test-model")
                .join("refs")
                .join("main"),
        )
        .expect("read ref"),
        "abc123"
    );
    assert_eq!(
        downloaded,
        cache_dir
            .path()
            .join("models--test-model")
            .join("snapshots")
            .join("abc123")
            .join("onnx")
            .join("model.onnx")
    );
}
