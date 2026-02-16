use crate::llm::openai::{Message, MessageContent};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── Mock SSE Server Helpers ─────────────────────────────────

/// Build an SSE `data:` line for a streaming chat chunk.
fn sse_chunk(content: &str) -> String {
    let json = serde_json::json!({
        "choices": [{
            "delta": { "content": content },
            "finish_reason": null
        }]
    });
    format!("data: {}\n\n", json)
}

fn sse_done() -> String {
    "data: [DONE]\n\n".to_string()
}

/// Build an SSE response body from a list of token strings.
fn build_sse_body(tokens: &[&str]) -> String {
    let mut body = String::new();
    for token in tokens {
        body.push_str(&sse_chunk(token));
    }
    body.push_str(&sse_done());
    body
}

// ── Multimodal Payload Structure ────────────────────────────

#[tokio::test]
async fn test_multimodal_payload_structure() {
    let content = MessageContent::with_images(
        "What is in this image?".to_string(),
        vec!["http://127.0.0.1:12345/vision/image_1_abc.png".to_string()],
    );

    let msg = Message {
        role: "user".to_string(),
        content,
    };

    let json = serde_json::to_value(&msg).unwrap();
    let content_arr = json["content"].as_array().unwrap();

    assert_eq!(content_arr.len(), 2, "should have text + 1 image part");
    assert_eq!(content_arr[0]["type"], "text");
    assert_eq!(content_arr[0]["text"], "What is in this image?");
    assert_eq!(content_arr[1]["type"], "image_url");
    assert!(content_arr[1]["image_url"]["url"]
        .as_str()
        .unwrap()
        .contains("/vision/image_1_abc.png"),);
}

#[tokio::test]
async fn test_multimodal_with_multiple_images() {
    let content = MessageContent::with_images(
        "Compare these images".to_string(),
        vec![
            "http://127.0.0.1:9999/vision/img1.png".to_string(),
            "http://127.0.0.1:9999/vision/img2.jpg".to_string(),
            "http://127.0.0.1:9999/vision/img3.webp".to_string(),
        ],
    );

    let json = serde_json::to_value(&content).unwrap();
    let parts = json.as_array().unwrap();
    assert_eq!(parts.len(), 4, "text + 3 images");

    for part in &parts[1..] {
        assert_eq!(part["type"], "image_url");
        assert!(part["image_url"]["url"].is_string());
    }
}

#[test]
fn test_message_content_text_extraction() {
    let content = MessageContent::with_images(
        "Describe this".to_string(),
        vec!["http://localhost/img.png".to_string()],
    );
    assert_eq!(content.text(), "Describe this");
}

// ── SSE Parsing via Direct HTTP (avoids system proxy issues) ─

/// Helper: make a no-proxy reqwest client, POST to mock server, and stream SSE.
/// This tests the same SSE format that OpenAIClient would use, but bypasses
/// the system proxy that may interfere with wiremock localhost connections.
async fn stream_from_mock(mock_server: &MockServer, sse_body: &str) -> Vec<String> {
    use eventsource_stream::Eventsource;
    use futures::StreamExt;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(sse_body.to_string())
                .insert_header("content-type", "text/event-stream"),
        )
        .mount(mock_server)
        .await;

    let client = reqwest::Client::builder().no_proxy().build().unwrap();

    let response = client
        .post(&format!("{}/v1/chat/completions", mock_server.uri()))
        .header("Content-Type", "application/json")
        .body("{}")
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success(), "mock should return 200");

    let mut tokens = Vec::new();
    let mut stream = response.bytes_stream().eventsource();

    while let Some(event_result) = stream.next().await {
        match event_result {
            Ok(event) => {
                if event.data == "[DONE]" {
                    break;
                }
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&event.data) {
                    if let Some(content) = parsed["choices"][0]["delta"]["content"].as_str() {
                        tokens.push(content.to_string());
                    }
                }
            }
            Err(_) => {} // Ignore parse errors (like OpenAIClient does)
        }
    }

    tokens
}

// ── Stream Chat with Mock Server ────────────────────────────

#[tokio::test]
async fn test_stream_chat_with_mock_server() {
    let mock_server = MockServer::start().await;
    let body = build_sse_body(&["Hello", " world", "!"]);
    let tokens = stream_from_mock(&mock_server, &body).await;
    assert_eq!(tokens.join(""), "Hello world!");
}

// ── Empty Response ──────────────────────────────────────────

#[tokio::test]
async fn test_mock_empty_response() {
    let mock_server = MockServer::start().await;
    let body = sse_done();
    let tokens = stream_from_mock(&mock_server, &body).await;
    assert_eq!(tokens.len(), 0, "empty response should produce no tokens");
}

// ── Malformed SSE ───────────────────────────────────────────

#[tokio::test]
async fn test_mock_malformed_sse() {
    let mock_server = MockServer::start().await;

    let valid_chunk = serde_json::json!({
        "choices": [{"delta": {"content": "recovered"}, "finish_reason": null}]
    });
    let body = format!(
        "data: {{\"invalid\n\ndata: {}\n\n{}",
        valid_chunk,
        sse_done()
    );

    let tokens = stream_from_mock(&mock_server, &body).await;

    assert!(
        tokens.iter().any(|t| t.contains("recovered")),
        "should recover valid tokens after malformed SSE, got: {:?}",
        tokens
    );
}

// ── Full Vision → Mock LLM Pipeline ────────────────────────

#[tokio::test]
async fn test_full_vision_to_mock_llm_pipeline() {
    use super::helpers::*;
    use eventsource_stream::Eventsource;
    use futures::StreamExt;

    // 1. Start vision server
    let (server, _tmp) = start_test_server().await;
    let png = make_png_bytes(256);

    // 2. Upload image
    let image_url = server.upload(&png, "pipeline_test.png").unwrap();

    // 3. Build multimodal message
    let content =
        MessageContent::with_images("What do you see?".to_string(), vec![image_url.clone()]);

    let messages = vec![
        Message {
            role: "system".to_string(),
            content: MessageContent::Text("You are a helpful assistant.".to_string()),
        },
        Message {
            role: "user".to_string(),
            content,
        },
    ];

    // 4. Verify JSON structure
    let json = serde_json::to_value(&messages).unwrap();
    let user_msg = &json[1];
    let parts = user_msg["content"].as_array().unwrap();
    let img_part = &parts[1];
    assert_eq!(img_part["image_url"]["url"].as_str().unwrap(), &image_url);

    // 5. Stream from mock LLM using no-proxy client
    let mock_server = MockServer::start().await;
    let sse_body = build_sse_body(&["I see", " a test", " image."]);

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(sse_body)
                .insert_header("content-type", "text/event-stream"),
        )
        .mount(&mock_server)
        .await;

    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let response = client
        .post(&format!("{}/v1/chat/completions", mock_server.uri()))
        .header("Content-Type", "application/json")
        .json(&json)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success(), "mock LLM should return 200");

    let mut full = String::new();
    let mut stream = response.bytes_stream().eventsource();

    while let Some(event_result) = stream.next().await {
        match event_result {
            Ok(event) => {
                if event.data == "[DONE]" {
                    break;
                }
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&event.data) {
                    if let Some(content) = parsed["choices"][0]["delta"]["content"].as_str() {
                        full.push_str(content);
                    }
                }
            }
            Err(_) => {}
        }
    }

    assert_eq!(full, "I see a test image.");
}
