use witan_sdk::{Client, Error, ErrorSearchOptions, SearchOptions};
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(server: &MockServer) -> Client {
    Client::builder().base_url(server.uri()).api_key("").retries(2).build().unwrap()
}

#[tokio::test]
async fn search_error_sends_the_error_in_the_body() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search/error"))
        .and(body_json(serde_json::json!({ "error": "npm error code ENEEDAUTH", "limit": 3 })))
        .and(header("user-agent", format!("witan-sdk-rust/{}", witan_sdk::VERSION).as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"signature":"npm error code ENEEDAUTH","mode":"error","results":[{"id":"u1","title":"T","score":"78.00","priceMicro":"0","match":{"kind":"exact","similarity":1}}]}"#,
        ))
        .mount(&s)
        .await;
    let a = client(&s).search_error("npm error code ENEEDAUTH", Some(ErrorSearchOptions { limit: Some(3), ..Default::default() })).await.unwrap();
    let h = &a.results[0];
    assert_eq!(h.id, "u1");
    assert_eq!(h.score, Some(78.0));
    assert_eq!(h.matched.kind, "exact");
    assert_eq!(h.price_micro, Some(0.0));
}

#[tokio::test]
async fn search_sends_query_and_key() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("q", "redis throughput"))
        .and(query_param("mode", "keyword"))
        .and(header("authorization", "Bearer km_test"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"results":[{"id":"a","score":null},{"id":"b","score":73}]}"#))
        .mount(&s)
        .await;
    let c = Client::builder().base_url(s.uri()).api_key("km_test").build().unwrap();
    let hits = c.search("redis throughput", Some(SearchOptions { mode: Some("keyword".into()), ..Default::default() })).await.unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].score, None);
    assert_eq!(hits[1].score, Some(73.0));
}

#[tokio::test]
async fn read_payment_required() {
    let s = MockServer::start().await;
    Mock::given(path("/knowledge/x/full"))
        .respond_with(ResponseTemplate::new(402).set_body_string(r#"{"error":"payment required","price":"$0.01","pay":"https://witan.markets/paid/knowledge?id=x","freeWithKey":true}"#))
        .mount(&s)
        .await;
    match client(&s).read("x").await {
        Err(Error::PaymentRequired { price, pay, free_with_key, .. }) => {
            assert_eq!(price.as_deref(), Some("$0.01"));
            assert!(pay.unwrap().contains("paid/knowledge"));
            assert!(free_with_key);
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn retries_503_then_succeeds() {
    let s = MockServer::start().await;
    Mock::given(path("/knowledge/u1/full"))
        .respond_with(ResponseTemplate::new(503).insert_header("retry-after", "0"))
        .up_to_n_times(2)
        .mount(&s)
        .await;
    Mock::given(path("/knowledge/u1/full"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"id":"u1","title":"T","body":"B"}"#))
        .mount(&s)
        .await;
    let u = client(&s).read("u1").await.unwrap();
    assert_eq!(u.body, "B");
}

#[tokio::test]
async fn gives_up_after_retries_and_keeps_the_message() {
    let s = MockServer::start().await;
    Mock::given(path("/projects"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0").set_body_string(r#"{"error":"slow down"}"#))
        .expect(3)
        .mount(&s)
        .await;
    let e = client(&s).datasets().await.unwrap_err();
    assert_eq!(e.status(), Some(429));
    assert!(e.to_string().contains("slow down"));
}

#[tokio::test]
async fn no_retry_on_404_and_slug_is_one_segment() {
    let s = MockServer::start().await;
    Mock::given(path("/projects/a%2Fb"))
        .respond_with(ResponseTemplate::new(404).set_body_string(r#"{"error":"not found"}"#))
        .expect(1)
        .mount(&s)
        .await;
    let e = client(&s).dataset("a/b").await.unwrap_err();
    assert_eq!(e.status(), Some(404));
}

#[tokio::test]
async fn query_body() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/projects/agent-api-observatory/query"))
        .and(body_json(serde_json::json!({ "sql": "SELECT 1", "version": 79 })))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"columns":["x"],"rows":[[1]],"count":1}"#))
        .mount(&s)
        .await;
    let q = client(&s).query("agent-api-observatory", "SELECT 1", Some(79), None).await.unwrap();
    assert_eq!(q.columns, vec!["x"]);
    assert_eq!(q.count, 1);
}
