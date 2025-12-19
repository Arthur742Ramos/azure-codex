use codex_core::protocol::EventMsg;
use codex_core::protocol::Op;
use codex_protocol::user_input::UserInput;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_sse_sequence;
use core_test_support::responses::sse;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::json;
use wiremock::MockServer;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_resume_after_response_failed() {
    skip_if_no_network!();

    let server = MockServer::start().await;
    let failed = sse(vec![json!({
        "type": "response.failed",
        "response": {
            "id": "resp_failed"
        }
    })]);
    let recovered = sse(vec![
        ev_response_created("resp_ok"),
        ev_assistant_message("msg-1", "Recovered."),
        ev_completed("resp_ok"),
    ]);
    let mock = mount_sse_sequence(&server, vec![failed, recovered]).await;

    let mut test_codex = test_codex().with_config(|config| {
        config.model_provider.stream_max_retries = Some(0);
    });
    let codex = test_codex.build(&server).await.unwrap().codex;

    codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "Please continue.".to_string(),
            }],
        })
        .await
        .unwrap();

    wait_for_event(&codex, |ev| matches!(ev, EventMsg::TaskComplete(_))).await;

    let requests = mock.requests();
    assert_eq!(2, requests.len());
    let first_inputs = requests[0].message_input_texts("user");
    assert!(first_inputs.iter().any(|text| text == "Please continue."));
    let second_inputs = requests[1].message_input_texts("user");
    assert!(second_inputs.iter().any(|text| text == "Keep going"));
}
