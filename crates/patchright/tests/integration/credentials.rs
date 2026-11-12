use crate::test_server::TestServer;
use patchright_rs::{CredentialsCreateOptions, CredentialsGetOptions, StorageStateOptions};

#[tokio::test]
async fn test_virtual_credentials_crud() {
    let (_pw, browser, context) = crate::common::setup_context().await;

    let creds = context.credentials();
    creds.install().await.expect("install authenticator");

    // No credentials registered yet.
    assert!(creds.get(None).await.expect("get").is_empty());

    let created = creds
        .create("example.com", None)
        .await
        .expect("create credential");
    assert_eq!(created.rp_id, "example.com");
    assert!(!created.id.is_empty(), "authenticator should assign an id");

    let all = creds.get(None).await.expect("get all");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, created.id);

    // Filter by relying party.
    let filtered = creds
        .get(Some(CredentialsGetOptions::default().rp_id("example.com")))
        .await
        .expect("get filtered");
    assert_eq!(filtered.len(), 1);

    let none = creds
        .get(Some(CredentialsGetOptions::default().rp_id("other.test")))
        .await
        .expect("get filtered miss");
    assert!(none.is_empty(), "no credentials for other.test");

    creds.delete(&created.id).await.expect("delete");
    assert!(creds.get(None).await.expect("get after delete").is_empty());

    browser.close().await.expect("close");
}

const ASSERT_AND_READ_SIGN_COUNT: &str = r#"async () => {
    const challenge = crypto.getRandomValues(new Uint8Array(32));
    const cred = await navigator.credentials.get({
        publicKey: { challenge, rpId: 'localhost', userVerification: 'preferred' },
    });
    return new DataView(cred.response.authenticatorData).getUint32(33);
}"#;

#[tokio::test]
#[ignore = "patchright: evaluates in an isolated world by default; the test reads page globals (passes under .main_world())"]
async fn sign_counts_are_seeded_reported_and_survive_storage_state() {
    let server = TestServer::start().await;
    let (_pw, browser, context) = crate::common::setup_context().await;
    let creds = context.credentials();

    let fresh = creds
        .create("fresh.example.com", None)
        .await
        .expect("create");
    assert_eq!(fresh.sign_count, 0);
    let seeded = creds
        .create(
            "localhost",
            CredentialsCreateOptions::default().sign_count(41),
        )
        .await
        .expect("create seeded");
    assert_eq!(seeded.sign_count, 41);

    creds.install().await.expect("install authenticator");
    let page = context.new_page().await.expect("new page");
    // WebAuthn needs a secure context, which http://localhost is.
    page.goto(&server.url().replace("127.0.0.1", "localhost"), None)
        .await
        .expect("goto");
    for expected in [42, 43] {
        let reported: u32 = page
            .evaluate(ASSERT_AND_READ_SIGN_COUNT, None::<&()>)
            .await
            .expect("assertion ceremony");
        assert_eq!(reported, expected, "each assertion increments the counter");
    }
    let after = creds
        .get(CredentialsGetOptions::default().id(seeded.id.clone()))
        .await
        .expect("get seeded");
    assert_eq!(after[0].sign_count, 43);

    let state = context
        .storage_state(StorageStateOptions::default().credentials(true))
        .await
        .expect("storage_state");
    let restored = browser
        .new_context_with_options(
            patchright_rs::BrowserContextOptions::builder()
                .storage_state(state)
                .build(),
        )
        .await
        .expect("restore");
    let back = restored
        .credentials()
        .get(CredentialsGetOptions::default().id(seeded.id.clone()))
        .await
        .expect("get restored");
    assert_eq!(
        back[0].sign_count, 43,
        "the counter survives a save and restore"
    );

    browser.close().await.expect("close");
    server.shutdown();
}
