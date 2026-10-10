//! The edge (Cloudflare Workers) entry point, run natively: mount an app in
//! memory as the Worker host does, boot it, answer requests. The app is
//! `examples/cloudflare-worker`, the one deployed at cf.solisoft.net.
//!
//! One test, because mounting, the environment and the booted app are
//! process-wide — as they are in a Worker isolate.

use std::path::Path;

use solilang::serve::edge::{self, EdgeRequest};

/// A controller the example app does not have, mounted only here: it echoes
/// an upload back, to prove bytes cross the edge entry point intact both ways.
const PROBE_CONTROLLER: &str = r#"
class ProbeController < Controller
  # POST /probe
  def create(req)
    file = find_uploaded_file(req, "file")
    {
      "status": 200,
      "headers": {"Content-Type": file["content_type"], "X-Name": file["filename"], "X-Note": params["note"]},
      "body_base64": file["data"]
    }
  end

  # PATCH /probe
  def update(req)
    {"status": 200, "body": "patched #{params["title"]}"}
  end
end
"#;

fn example_files() -> Vec<(String, Vec<u8>)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/cloudflare-worker");
    let mut files = Vec::new();
    for dir in ["app", "config"] {
        for entry in walkdir::WalkDir::new(root.join(dir)) {
            let entry = entry.unwrap();
            if entry.file_type().is_file() {
                let relative = entry.path().strip_prefix(&root).unwrap();
                let key = format!("/app/{}", relative.to_string_lossy().replace('\\', "/"));
                files.push((key, std::fs::read(entry.path()).unwrap()));
            }
        }
    }
    for (path, contents) in files.iter_mut() {
        if path == "/app/config/routes.sl" {
            contents.extend_from_slice(
                b"\npost(\"/probe\", \"probe#create\")\npatch(\"/probe\", \"probe#update\")\n",
            );
        }
    }
    files.push((
        "/app/app/controllers/probe_controller.sl".to_string(),
        PROBE_CONTROLLER.as_bytes().to_vec(),
    ));
    files
}

fn post(content_type: &str, body: Vec<u8>) -> edge::EdgeResponse {
    edge::handle(EdgeRequest {
        method: "POST".to_string(),
        path: "/probe".to_string(),
        query: String::new(),
        headers: vec![
            ("host".to_string(), "cf.example.test".to_string()),
            ("content-type".to_string(), content_type.to_string()),
        ],
        body,
    })
}

fn get(path: &str, query: &str) -> (u16, String) {
    let response = edge::handle(EdgeRequest {
        method: "GET".to_string(),
        path: path.to_string(),
        query: query.to_string(),
        headers: vec![
            ("host".to_string(), "cf.example.test".to_string()),
            ("cf-colo".to_string(), "MRS".to_string()),
            ("cf-ipcountry".to_string(), "FR".to_string()),
        ],
        body: Vec::new(),
    });
    (response.status, String::from_utf8(response.body).unwrap())
}

#[test]
fn boots_a_mounted_app_and_answers_requests() {
    // The interpreter recurses deeply in debug builds; give it a Worker-sized stack.
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            // Before boot every request is a 503, not a panic.
            assert_eq!(get("/", "").0, 503);

            solilang::platform::fs::mount(example_files());
            solilang::platform::env::set(vec![
                ("SOLI_RUNTIME".to_string(), "cloudflare-workers".to_string()),
                ("SOLI_VERSION".to_string(), "test".to_string()),
            ]);
            edge::boot(Path::new("/app")).expect("boot");

            let (status, body) = get("/", "");
            assert_eq!(status, 200, "{body}");
            assert!(body.contains("Soli on Cloudflare Workers"), "{body}");
            // The layout wrapped the view, with the @runtime the action set.
            assert!(body.contains("<strong>MRS</strong>"), "{body}");

            let (status, body) = get("/try", "text=edge+soli+edge");
            assert_eq!(status, 200, "{body}");
            assert!(body.contains("<span>3</span> words"), "{body}");
            assert!(body.contains("<span>2</span> distinct"), "{body}");

            // getenv() reads the table the host set, not the process environment.
            let (status, body) = get("/api/info", "");
            assert_eq!(status, 200, "{body}");
            let info: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert_eq!(info["runtime"], "cloudflare-workers");
            assert_eq!(info["version"], "test");
            assert_eq!(info["colo"], "MRS");

            assert_eq!(get("/no/such/page", "").0, 404);

            // A multipart upload of bytes that are not UTF-8 reaches the action
            // whole, and an action answering bytes sends them back unchanged.
            let image: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
            let mut body = b"--XyZ\r\nContent-Disposition: form-data; name=\"note\"\r\n\r\nhello\r\n--XyZ\r\nContent-Disposition: form-data; name=\"file\"; filename=\"pixels.png\"\r\nContent-Type: image/png\r\n\r\n".to_vec();
            body.extend_from_slice(&image);
            body.extend_from_slice(b"\r\n--XyZ--\r\n");
            let response = post("multipart/form-data; boundary=XyZ", body);
            assert_eq!(response.status, 200, "{}", String::from_utf8_lossy(&response.body));
            assert_eq!(response.body, image);
            let header = |name: &str| {
                response
                    .headers
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(name))
                    .map(|(_, v)| v.clone())
            };
            assert_eq!(header("content-type").as_deref(), Some("image/png"));
            assert_eq!(header("x-name").as_deref(), Some("pixels.png"));
            assert_eq!(header("x-note").as_deref(), Some("hello"));

            // A form's `_method` field picks the verb, as under `soli serve`.
            let response = post(
                "application/x-www-form-urlencoded",
                b"_method=PATCH&title=edge".to_vec(),
            );
            assert_eq!(response.status, 200);
            assert_eq!(String::from_utf8(response.body).unwrap(), "patched edge");

            // The instant-navigation script every page links to.
            let (status, body) = get("/__soli/nav.js", "v=1");
            assert_eq!(status, 200);
            assert!(
                body.contains("soli:load"),
                "{}",
                &body[..80.min(body.len())]
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn refuses_to_boot_a_folder_that_is_not_an_app() {
    // Nothing is mounted in this process yet, so this reads the real disk.
    let err = edge::boot(Path::new("/definitely/not/a/soli/app")).unwrap_err();
    assert!(err.to_string().contains("not a Soli app"), "{err}");
}
