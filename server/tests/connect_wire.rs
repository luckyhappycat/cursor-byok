//! Verifies captured Cursor Connect framing and protobuf compatibility.
#[path = "support/fake_cursor.rs"]
mod fake_cursor;
#[path = "support/fake_provider.rs"]
mod fake_provider;
#[path = "support/fixtures.rs"]
mod fixtures;

use std::{io::Write, sync::Arc};

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine};
use cursor_server::{
    api::cursor,
    cursor::prompting::{PromptAssets, PromptCompiler},
    cursor::protocol::{
        connect,
        proto::{agent::v1 as pb, aiserver::v1 as ai},
    },
    cursor::transport::TransportRegistry,
    network::NetworkClients,
};
use flate2::{write::GzEncoder, Compression};
use prost::Message;
use tower::ServiceExt;

#[test]
fn connect_envelope_is_flag_plus_big_endian_length_plus_protobuf() {
    let message = pb::BidiRequestId {
        request_id: "abc".into(),
    };
    let frame = connect::encode_message(&message).unwrap();
    assert_eq!(&frame[..5], &[0, 0, 0, 0, 5]);
    let decoded: pb::BidiRequestId = fake_cursor::decode_single(&frame).unwrap();
    assert_eq!(decoded.request_id, "abc");
}

#[test]
fn end_stream_matches_captured_connect_shape() {
    assert_eq!(
        connect::encode_end_stream().as_ref(),
        &[2, 0, 0, 0, 2, b'{', b'}']
    );
}

#[test]
fn error_end_stream_is_flagged_json_not_protobuf() {
    let frame = connect::encode_error_end_stream(&connect::ConnectStreamError {
        code: connect::ConnectCode::Unavailable,
        message: "overloaded".into(),
        details: vec![connect::ConnectErrorDetail {
            type_name: "aiserver.v1.ErrorDetails".into(),
            value: "AQ".into(),
        }],
    })
    .unwrap();
    let (flags, payload) = connect::decode_frames(&frame).unwrap().pop().unwrap();
    assert_eq!(flags, connect::END_STREAM_FLAG);
    let json: serde_json::Value = serde_json::from_slice(&payload).unwrap();
    assert_eq!(json["error"]["code"], "unavailable");
    assert_eq!(json["error"]["message"], "overloaded");
    assert_eq!(
        json["error"]["details"][0]["type"],
        "aiserver.v1.ErrorDetails"
    );
}

#[test]
fn cursor_error_details_subset_decodes_captured_wire_value() {
    let captured = "CAISVQoUQXV0aGVudGljYXRpb24gZXJyb3ISMklmIHlvdSBhcmUgbG9nZ2VkIGluLCB0cnkgbG9nZ2luZyBvdXQgYW5kIGJhY2sgaW4uIABSBwoFbG9naW4YAQ";
    let bytes = STANDARD_NO_PAD.decode(captured).unwrap();
    let details = ai::ErrorDetails::decode(bytes.as_slice()).unwrap();
    assert_eq!(details.error, 2, "ERROR_NOT_LOGGED_IN");
    assert_eq!(details.is_expected, Some(true));
    let custom = details.details.unwrap();
    assert_eq!(custom.title, "Authentication error");
    assert_eq!(custom.is_retryable, Some(false));
}

#[test]
fn captured_kv_ack_hex_decodes_as_agent_client_message() {
    let bytes = hex::decode("1a0408011a00").unwrap();
    let message = pb::AgentClientMessage::decode(bytes.as_slice()).unwrap();
    let Some(pb::agent_client_message::Message::KvClientMessage(kv)) = message.message else {
        panic!("expected KV client message")
    };
    assert_eq!(kv.id, 1);
    assert!(matches!(
        kv.message,
        Some(pb::kv_client_message::Message::SetBlobResult(_))
    ));
}

#[test]
fn bidi_append_decodes_binary_and_hex_agent_messages() {
    let payload = hex::decode("1a0408011a00").unwrap();
    for binary in [true, false] {
        let request = ai::BidiAppendRequest {
            request_id: Some(ai::BidiRequestId {
                request_id: "payload-request".into(),
            }),
            append_seqno: 7,
            data_binary: if binary { payload.clone() } else { Vec::new() },
            data: if binary {
                String::new()
            } else {
                hex::encode(&payload)
            },
        };
        let decoded = cursor::bidi::decode(&request).unwrap();
        assert_eq!(decoded.request_id, "payload-request");
        assert_eq!(decoded.seqno, 7);
        assert_eq!(decoded.message.encode_to_vec(), payload);
    }
}

#[test]
fn bidi_append_uses_binary_payload_when_both_fields_are_present() {
    let payload = hex::decode("1a0408011a00").unwrap();
    let request = ai::BidiAppendRequest {
        request_id: Some(ai::BidiRequestId {
            request_id: "binary-request".into(),
        }),
        data: "invalid hex".into(),
        data_binary: payload.clone(),
        ..Default::default()
    };
    let decoded = cursor::bidi::decode(&request).unwrap();
    assert_eq!(decoded.message.encode_to_vec(), payload);
}

#[test]
fn bidi_append_rejects_malformed_binary_payload() {
    let request = ai::BidiAppendRequest {
        request_id: Some(ai::BidiRequestId {
            request_id: "malformed-request".into(),
        }),
        data_binary: vec![0xff],
        ..Default::default()
    };
    let error = cursor::bidi::decode(&request).err().unwrap();
    assert!(error.to_string().contains("protobuf decode error"));
}

#[tokio::test]
async fn binary_bidi_append_accepts_plain_and_gzip_http_requests() {
    let (_directory, store) = fixtures::temp_store().await;
    let clients = NetworkClients::new(store.clone());
    let registry = TransportRegistry::new(
        store,
        Arc::new(fake_provider::FakeProvider::default()),
        PromptCompiler::new(PromptAssets::embedded().unwrap()),
    );
    let router = cursor::router(registry.clone(), clients).unwrap();
    for gzip in [false, true] {
        let request_id = format!("binary-http-{gzip}");
        let handle = registry.get_or_create(&request_id).await.unwrap();
        let mut output = handle.subscribe();
        let wire = ai::BidiAppendRequest {
            request_id: Some(ai::BidiRequestId { request_id }),
            data_binary: pb::AgentClientMessage {
                message: Some(pb::agent_client_message::Message::ClientHeartbeat(
                    pb::ClientHeartbeat {},
                )),
            }
            .encode_to_vec(),
            ..Default::default()
        }
        .encode_to_vec();
        let mut request = Request::post("/aiserver.v1.BidiService/BidiAppend")
            .header(header::CONTENT_TYPE, "application/proto");
        let body = if gzip {
            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(&wire).unwrap();
            request = request.header(header::CONTENT_ENCODING, "gzip");
            encoder.finish().unwrap()
        } else {
            wire
        };
        let response = router
            .clone()
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let frame = tokio::time::timeout(std::time::Duration::from_secs(1), output.recv())
            .await
            .unwrap()
            .unwrap();
        let message: pb::AgentServerMessage = fake_cursor::decode_single(&frame).unwrap();
        let Some(pb::agent_server_message::Message::InteractionUpdate(update)) = message.message
        else {
            panic!("expected interaction update")
        };
        assert!(matches!(
            update.message,
            Some(pb::interaction_update::Message::Heartbeat(_))
        ));
    }
    registry.shutdown().await;
}

#[tokio::test]
async fn bidi_append_gzip_body_is_decompressed_before_protobuf_decode() {
    let (_directory, store) = fixtures::temp_store().await;
    let assets = PromptAssets::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("prompt/cursor")
            .as_path(),
    )
    .unwrap();
    let clients = NetworkClients::new(store.clone());
    let registry = TransportRegistry::new(
        store,
        Arc::new(fake_provider::FakeProvider::default()),
        PromptCompiler::new(assets),
    );
    let wire = ai::BidiAppendRequest {
        request_id: Some(ai::BidiRequestId {
            request_id: "gzip-request".into(),
        }),
        ..Default::default()
    }
    .encode_to_vec();
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&wire).unwrap();
    let compressed = encoder.finish().unwrap();

    let response = cursor::router(registry, clients)
        .unwrap()
        .oneshot(
            Request::post("/aiserver.v1.BidiService/BidiAppend")
                .header(header::CONTENT_TYPE, "application/proto")
                .header(header::CONTENT_ENCODING, "gzip")
                .body(Body::from(compressed))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let text = std::str::from_utf8(&body).unwrap();
    assert!(text.contains("BidiAppend contains no AgentClientMessage"));
    assert!(!text.contains("protobuf decode error"));
}
