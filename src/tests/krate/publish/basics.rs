use crate::schema::versions_published_by;
use crate::tests::builders::{CrateBuilder, PublishBuilder};
use crate::tests::util::{RequestHelper, TestApp};
use diesel::QueryDsl;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use googletest::prelude::*;
use insta::{assert_json_snapshot, assert_snapshot};

#[tokio::test(flavor = "multi_thread")]
async fn new_krate() {
    let (app, _, user) = TestApp::full().with_user().await;
    let mut conn = app.db_conn().await;

    let crate_to_publish = PublishBuilder::new("foo_new", "1.0.0");
    let response = user.publish_crate(crate_to_publish).await;
    assert_snapshot!(response.status(), @"200 OK");
    assert_json_snapshot!(response.json(), {
        ".crate.created_at" => "[datetime]",
        ".crate.updated_at" => "[datetime]",
    });

    let crates = app.crates_from_index_head("foo_new");
    assert_json_snapshot!(crates);

    assert_snapshot!(app.stored_files().await.join("\n"), @r"
    crates/foo_new/foo_new-1.0.0.crate
    index/fo/o_/foo_new
    rss/crates.xml
    rss/crates/foo_new.xml
    rss/updates.xml
    ");

    let email: String = versions_published_by::table
        .select(versions_published_by::email)
        .first(&mut conn)
        .await
        .unwrap();
    assert_eq!(email, "foo@example.com");

    assert_snapshot!(app.emails_snapshot().await);
}

#[tokio::test(flavor = "multi_thread")]
async fn new_krate_with_token() {
    let (app, _, _, token) = TestApp::full().with_token().await;

    let crate_to_publish = PublishBuilder::new("foo_new", "1.0.0");
    let response = token.publish_crate(crate_to_publish).await;
    assert_snapshot!(response.status(), @"200 OK");
    assert_json_snapshot!(response.json(), {
        ".crate.created_at" => "[datetime]",
        ".crate.updated_at" => "[datetime]",
    });

    assert_snapshot!(app.stored_files().await.join("\n"), @r"
    crates/foo_new/foo_new-1.0.0.crate
    index/fo/o_/foo_new
    rss/crates.xml
    rss/crates/foo_new.xml
    rss/updates.xml
    ");
}

#[tokio::test(flavor = "multi_thread")]
async fn new_krate_weird_version() {
    let (app, _, _, token) = TestApp::full().with_token().await;

    let crate_to_publish = PublishBuilder::new("foo_weird", "0.0.0-pre");
    let response = token.publish_crate(crate_to_publish).await;
    assert_snapshot!(response.status(), @"200 OK");
    assert_json_snapshot!(response.json(), {
        ".crate.created_at" => "[datetime]",
        ".crate.updated_at" => "[datetime]",
    });

    assert_snapshot!(app.stored_files().await.join("\n"), @r"
    crates/foo_weird/foo_weird-0.0.0-pre.crate
    index/fo/o_/foo_weird
    rss/crates.xml
    rss/crates/foo_weird.xml
    rss/updates.xml
    ");
}

#[tokio::test(flavor = "multi_thread")]
async fn new_krate_twice() {
    let (app, _, _, token) = TestApp::full().with_token().await;

    let crate_to_publish = PublishBuilder::new("foo_twice", "0.99.0");
    token.publish_crate(crate_to_publish).await.good();

    let crate_to_publish =
        PublishBuilder::new("foo_twice", "2.0.0").description("2.0.0 description");
    let response = token.publish_crate(crate_to_publish).await;
    assert_snapshot!(response.status(), @"200 OK");
    assert_json_snapshot!(response.json(), {
        ".crate.created_at" => "[datetime]",
        ".crate.updated_at" => "[datetime]",
    });

    let crates = app.crates_from_index_head("foo_twice");
    assert_json_snapshot!(crates);

    assert_snapshot!(app.stored_files().await.join("\n"), @r"
    crates/foo_twice/foo_twice-0.99.0.crate
    crates/foo_twice/foo_twice-2.0.0.crate
    index/fo/o_/foo_twice
    rss/crates.xml
    rss/crates/foo_twice.xml
    rss/updates.xml
    ");
}

// This is similar to the `new_krate_twice` case, but the versions are published in reverse order.
// The primary purpose is to verify that the `default_version` we provide is as expected.
#[tokio::test(flavor = "multi_thread")]
async fn new_krate_twice_alt() {
    let (app, _, _, token) = TestApp::full().with_token().await;

    let crate_to_publish =
        PublishBuilder::new("foo_twice", "2.0.0").description("2.0.0 description");
    token.publish_crate(crate_to_publish).await.good();

    let crate_to_publish = PublishBuilder::new("foo_twice", "0.99.0");
    let response = token.publish_crate(crate_to_publish).await;
    assert_snapshot!(response.status(), @"200 OK");
    assert_json_snapshot!(response.json(), {
        ".crate.created_at" => "[datetime]",
        ".crate.updated_at" => "[datetime]",
    });

    let crates = app.crates_from_index_head("foo_twice");
    assert_json_snapshot!(crates);

    assert_snapshot!(app.stored_files().await.join("\n"), @r"
    crates/foo_twice/foo_twice-0.99.0.crate
    crates/foo_twice/foo_twice-2.0.0.crate
    index/fo/o_/foo_twice
    rss/crates.xml
    rss/crates/foo_twice.xml
    rss/updates.xml
    ");
}

#[tokio::test(flavor = "multi_thread")]
async fn new_krate_duplicate_version() {
    let (app, _, user, token) = TestApp::full().with_token().await;
    let mut conn = app.db_conn().await;

    // Insert a crate directly into the database and then we'll try to publish the same version
    CrateBuilder::new("foo_dupe", user.as_model().id)
        .version("1.0.0")
        .expect_build(&mut conn)
        .await;

    let crate_to_publish = PublishBuilder::new("foo_dupe", "1.0.0");
    let response = token.publish_crate(crate_to_publish).await;
    assert_snapshot!(response.status(), @"400 Bad Request");
    assert_snapshot!(response.text(), @r#"{"errors":[{"detail":"crate version `1.0.0` is already uploaded"}]}"#);

    assert_that!(app.stored_files().await, empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn new_krate_advance_num_versions() {
    use crate::schema::default_versions;

    let (app, _, _, token) = TestApp::full().with_token().await;
    let mut conn = app.db_conn().await;

    async fn assert_num_versions(conn: &mut AsyncPgConnection, expected: i32) {
        let num_versions = default_versions::table
            .select(default_versions::num_versions)
            .load::<Option<i32>>(conn)
            .await
            .unwrap();
        assert_eq!(num_versions.len(), 1);
        assert_eq!(num_versions[0], Some(expected));
    }

    let crate_to_publish = PublishBuilder::new("foo", "2.0.0").description("2.0.0 description");
    token.publish_crate(crate_to_publish).await.good();
    assert_num_versions(&mut conn, 1).await;

    let crate_to_publish = PublishBuilder::new("foo", "2.0.1").description("2.0.1 description");
    token.publish_crate(crate_to_publish).await.good();
    assert_num_versions(&mut conn, 2).await;

    let crate_to_publish = PublishBuilder::new("foo", "2.0.2").description("2.0.2 description");
    token.publish_crate(crate_to_publish).await.good();
    assert_num_versions(&mut conn, 3).await;
}
