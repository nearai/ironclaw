use std::path::Path;
use std::process::{Command, Output};
use std::sync::Arc;
use std::time::Duration;

use tempfile::tempdir;

const AUTH_TOKEN: &str = "anp-test-token";

fn run_cli(base_dir: &Path, args: &[&str]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_ironclaw"))
        .args(args)
        .env("IRONCLAW_BASE_DIR", base_dir)
        .env_remove("DATABASE_URL")
        .env_remove("DATABASE_BACKEND")
        .env_remove("LIBSQL_PATH")
        .env_remove("LIBSQL_URL")
        .env_remove("SECRETS_MASTER_KEY")
        .env_remove("OPENAI_API_KEY")
        .output()
        .expect("run ironclaw CLI");

    assert!(
        output.status.success(),
        "command {:?} failed:\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("reqwest client")
}

#[test]
fn did_cli_generates_stable_identity_and_document() {
    let dir = tempdir().expect("tempdir");
    let identity_path = dir.path().join("identity").join("instance.json");

    let first = run_cli(dir.path(), &["--no-db", "did", "show"]);
    let did = String::from_utf8(first.stdout)
        .expect("utf8 DID output")
        .trim()
        .to_string();

    assert!(did.starts_with("did:key:z"));
    assert!(identity_path.exists(), "identity file should be created");

    let second = run_cli(dir.path(), &["--no-db", "did", "show"]);
    let did_second = String::from_utf8(second.stdout)
        .expect("utf8 DID output")
        .trim()
        .to_string();
    assert_eq!(did, did_second, "DID should persist across invocations");

    let document = run_cli(dir.path(), &["--no-db", "did", "document"]);
    let json: serde_json::Value =
        serde_json::from_slice(&document.stdout).expect("valid DID document JSON");

    assert_eq!(json["id"], did);
    assert_eq!(json["verificationMethod"][0]["controller"], did);
}

#[test]
fn status_command_shows_did_and_identity_path() {
    let dir = tempdir().expect("tempdir");
    let identity_path = dir.path().join("identity").join("instance.json");

    let first = run_cli(dir.path(), &["--no-db", "did", "show"]);
    let did = String::from_utf8(first.stdout)
        .expect("utf8 DID output")
        .trim()
        .to_string();

    let status = run_cli(dir.path(), &["--no-db", "status"]);
    let stdout = String::from_utf8(status.stdout).expect("utf8 status output");

    assert!(stdout.contains("DID:"));
    assert!(stdout.contains(&did));
    assert!(stdout.contains("DID Path:"));
    assert!(stdout.contains(&identity_path.display().to_string()));
}

#[tokio::test]
async fn gateway_identity_endpoints_are_protected_and_return_expected_metadata() {
    let dir = tempdir().expect("tempdir");
    let identity = Arc::new(
        ironclaw::did::load_or_create_at(&dir.path().join("instance.json")).expect("identity"),
    );
    let (addr, _state) = ironclaw::channels::web::test_helpers::TestGatewayBuilder::new()
        .instance_identity(Arc::clone(&identity))
        .agent_name("ANP Test Agent")
        .start(AUTH_TOKEN)
        .await
        .expect("start gateway");

    let client = client();

    let unauthorized = client
        .get(format!("http://{addr}/api/identity"))
        .send()
        .await
        .expect("send unauthorized request");
    assert_eq!(unauthorized.status(), reqwest::StatusCode::UNAUTHORIZED);

    let identity_resp: serde_json::Value = client
        .get(format!("http://{addr}/api/identity"))
        .bearer_auth(AUTH_TOKEN)
        .send()
        .await
        .expect("send authorized identity request")
        .error_for_status()
        .expect("identity endpoint ok")
        .json()
        .await
        .expect("identity JSON");
    assert_eq!(identity_resp["did"], identity.did());
    assert_eq!(identity_resp["method"], "did:key");
    assert_eq!(identity_resp["key_id"], identity.key_id());

    let did_document: serde_json::Value = client
        .get(format!("http://{addr}/api/identity/did-document"))
        .bearer_auth(AUTH_TOKEN)
        .send()
        .await
        .expect("send DID document request")
        .error_for_status()
        .expect("did document endpoint ok")
        .json()
        .await
        .expect("did document JSON");
    assert_eq!(did_document["id"], identity.did());
    assert_eq!(did_document["authentication"][0], identity.key_id());

    let agent_description: serde_json::Value = client
        .get(format!("http://{addr}/api/identity/agent-description"))
        .bearer_auth(AUTH_TOKEN)
        .send()
        .await
        .expect("send agent description request")
        .error_for_status()
        .expect("agent description endpoint ok")
        .json()
        .await
        .expect("agent description JSON");
    assert_eq!(agent_description["protocolType"], "ANP");
    assert_eq!(agent_description["type"], "AgentDescription");
    assert_eq!(agent_description["name"], "ANP Test Agent");
    assert_eq!(agent_description["did"], identity.did());

    let public_route = client
        .get(format!("http://{addr}/.well-known/agent-descriptions"))
        .send()
        .await
        .expect("send public discovery request");
    assert_eq!(public_route.status(), reqwest::StatusCode::NOT_FOUND);
}
