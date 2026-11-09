//! Making pictures: OpenRouter image models, and ComfyUI running a workflow.

use std::time::Duration;

use lucy_providers::ProviderError;
use lucy_providers::generate::{
    ComfyUi, ComfyUiConfig, ImageGenerator, ImageRequest, OpenRouterImages,
};
use lucy_providers::openai_compat::OpenAiCompatConfig;
use lucy_providers::testing::{FakeResponse, FakeServer};
use serde_json::{Value, json};

/// A tiny PNG, base64.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYGBgAAAABQABpfZFQAAAAABJRU5ErkJggg==";

fn request(prompt: &str) -> ImageRequest {
    ImageRequest {
        prompt: prompt.into(),
        negative: None,
        width: 1024,
        height: 1024,
    }
}

// --- OpenRouter ---------------------------------------------------------------------------

fn openrouter(server: &FakeServer) -> OpenRouterImages {
    OpenRouterImages::new(OpenAiCompatConfig {
        name: "pics".into(),
        base_url: format!("{}/api/v1", server.url()),
        model: "maker/image-1".into(),
        api_key: Some("sk-or-test".into()),
    })
}

fn picture_reply() -> FakeResponse {
    FakeResponse::json(
        200,
        json!({"choices": [{"message": {"role": "assistant", "content": "Here you go.",
            "images": [{"type": "image_url",
                        "image_url": {"url": format!("data:image/png;base64,{PNG}")}}]}}]})
        .to_string(),
    )
}

#[tokio::test]
async fn asks_an_openrouter_image_model_for_a_picture() {
    let server = FakeServer::start([picture_reply()]).await;

    let pictures = openrouter(&server)
        .generate(&ImageRequest {
            prompt: "a lighthouse at dusk".into(),
            negative: Some("people".into()),
            width: 1344,
            height: 768,
        })
        .await
        .unwrap();

    assert_eq!(pictures.len(), 1);
    assert_eq!(pictures[0].media_type, "image/png");
    assert_eq!(pictures[0].data, PNG);
    let [sent] = server.requests().try_into().unwrap();
    assert_eq!(sent.path, "/api/v1/chat/completions");
    assert_eq!(sent.headers["authorization"], "Bearer sk-or-test");
    assert_eq!(sent.body["model"], "maker/image-1");
    assert_eq!(sent.body["modalities"], json!(["image", "text"]));
    assert_eq!(sent.body["image_config"]["aspect_ratio"], "16:9");
    let prompt = sent.body["messages"][0]["content"].as_str().unwrap();
    assert!(prompt.contains("a lighthouse at dusk"), "{prompt}");
    assert!(prompt.contains("people"), "{prompt}");
}

#[tokio::test]
async fn image_only_models_are_asked_again_for_just_a_picture() {
    let refused = FakeResponse::json(
        400,
        json!({"error": {"message": "No endpoints found that support the requested output modalities: image, text"}})
            .to_string(),
    );
    let server = FakeServer::start([refused, picture_reply()]).await;

    let pictures = openrouter(&server)
        .generate(&request("a fox"))
        .await
        .unwrap();

    assert_eq!(pictures.len(), 1);
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].body["modalities"], json!(["image"]));
}

#[tokio::test]
async fn a_reply_without_a_picture_says_what_the_model_said_instead() {
    let words = FakeResponse::json(
        200,
        json!({"choices": [{"message": {"role": "assistant", "content": "I can't draw that."}}]})
            .to_string(),
    );
    let server = FakeServer::start([words]).await;

    let err = openrouter(&server)
        .generate(&request("a fox"))
        .await
        .unwrap_err();

    assert!(matches!(err, ProviderError::Refused(_)), "{err:?}");
    assert!(err.to_string().contains("I can't draw that."), "{err}");
}

// --- ComfyUI ------------------------------------------------------------------------------

fn comfyui(server: &FakeServer, workflow: Value, checkpoint: Option<&str>) -> ComfyUi {
    ComfyUi::new(ComfyUiConfig {
        url: server.url().into(),
        workflow,
        checkpoint: checkpoint.map(Into::into),
        poll_every: Duration::from_millis(5),
    })
}

/// A small workflow using every placeholder.
fn workflow() -> Value {
    json!({
        "4": {"class_type": "CheckpointLoaderSimple", "inputs": {"ckpt_name": "{{checkpoint}}"}},
        "5": {"class_type": "EmptyLatentImage",
              "inputs": {"width": "{{width}}", "height": "{{height}}", "batch_size": 1}},
        "6": {"class_type": "CLIPTextEncode", "inputs": {"text": "{{prompt}}, high detail"}},
        "7": {"class_type": "CLIPTextEncode", "inputs": {"text": "{{negative}}"}},
        "3": {"class_type": "KSampler", "inputs": {"seed": "{{seed}}", "steps": 20}},
        "9": {"class_type": "SaveImage", "inputs": {"filename_prefix": "Lucy"}},
    })
}

fn queued() -> FakeResponse {
    FakeResponse::json(200, json!({"prompt_id": "p-1", "number": 0}).to_string())
}

fn still_running() -> FakeResponse {
    FakeResponse::json(200, "{}")
}

fn finished() -> FakeResponse {
    FakeResponse::json(
        200,
        json!({"p-1": {
            "outputs": {"9": {"images": [
                {"filename": "Lucy_00001_.png", "subfolder": "", "type": "output"}]}},
            "status": {"status_str": "success", "completed": true, "messages": []},
        }})
        .to_string(),
    )
}

fn png_bytes() -> FakeResponse {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(PNG)
        .unwrap();
    FakeResponse::body(200, "image/png", bytes)
}

#[tokio::test]
async fn comfyui_runs_the_workflow_with_the_placeholders_filled_in() {
    let server = FakeServer::start([queued(), still_running(), finished(), png_bytes()]).await;

    let pictures = comfyui(&server, workflow(), Some("dream.safetensors"))
        .generate(&ImageRequest {
            prompt: "a \"quoted\" lighthouse".into(),
            negative: Some("blurry".into()),
            width: 832,
            height: 1216,
        })
        .await
        .unwrap();

    assert_eq!(pictures.len(), 1);
    assert_eq!(pictures[0].media_type, "image/png");
    assert_eq!(pictures[0].data, PNG);
    let requests = server.requests();
    let paths: Vec<&str> = requests.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "/prompt",
            "/history/p-1",
            "/history/p-1",
            "/view?filename=Lucy_00001_.png&subfolder=&type=output",
        ]
    );
    let graph = &requests[0].body["prompt"];
    assert_eq!(graph["4"]["inputs"]["ckpt_name"], "dream.safetensors");
    assert_eq!(graph["5"]["inputs"]["width"], 832);
    assert_eq!(graph["5"]["inputs"]["height"], 1216);
    assert_eq!(graph["5"]["inputs"]["batch_size"], 1);
    assert_eq!(
        graph["6"]["inputs"]["text"],
        "a \"quoted\" lighthouse, high detail"
    );
    assert_eq!(graph["7"]["inputs"]["text"], "blurry");
    assert!(graph["3"]["inputs"]["seed"].is_u64(), "{}", graph["3"]);
}

#[tokio::test]
async fn comfyui_uses_its_first_checkpoint_when_none_is_set() {
    let checkpoints = FakeResponse::json(
        200,
        json!({"CheckpointLoaderSimple": {"input": {"required": {
            "ckpt_name": [["first.safetensors", "second.safetensors"], {}]}}}})
        .to_string(),
    );
    let server = FakeServer::start([checkpoints, queued(), finished(), png_bytes()]).await;

    comfyui(&server, workflow(), None)
        .generate(&request("a fox"))
        .await
        .unwrap();

    let requests = server.requests();
    assert_eq!(requests[0].path, "/object_info/CheckpointLoaderSimple");
    assert_eq!(
        requests[1].body["prompt"]["4"]["inputs"]["ckpt_name"],
        "first.safetensors"
    );
}

#[tokio::test]
async fn comfyui_explains_a_workflow_it_rejects() {
    let rejected = FakeResponse::json(
        400,
        json!({
            "error": {"type": "prompt_outputs_failed_validation",
                      "message": "Prompt outputs failed validation"},
            "node_errors": {"4": {"errors": [{"message": "Value not in list",
                "details": "ckpt_name: 'missing.safetensors' not in []"}],
                "class_type": "CheckpointLoaderSimple"}},
        })
        .to_string(),
    );
    let server = FakeServer::start([rejected]).await;

    let err = comfyui(&server, workflow(), Some("missing.safetensors"))
        .generate(&request("a fox"))
        .await
        .unwrap_err();

    let text = err.to_string();
    assert!(text.contains("Prompt outputs failed validation"), "{text}");
    assert!(text.contains("missing.safetensors"), "{text}");
}

#[tokio::test]
async fn comfyui_reports_a_run_that_failed() {
    let failed = FakeResponse::json(
        200,
        json!({"p-1": {
            "outputs": {},
            "status": {"status_str": "error", "completed": false, "messages": [
                ["execution_error", {"node_type": "KSampler",
                                     "exception_message": "CUDA out of memory"}]]},
        }})
        .to_string(),
    );
    let server = FakeServer::start([queued(), failed]).await;

    let err = comfyui(&server, workflow(), Some("dream.safetensors"))
        .generate(&request("a fox"))
        .await
        .unwrap_err();

    assert!(err.to_string().contains("CUDA out of memory"), "{err}");
}

#[test]
fn the_default_workflow_has_the_placeholders() {
    let workflow: Value = serde_json::from_str(lucy_providers::generate::DEFAULT_WORKFLOW).unwrap();
    let text = workflow.to_string();
    for placeholder in [
        "{{prompt}}",
        "{{negative}}",
        "{{width}}",
        "{{height}}",
        "{{seed}}",
    ] {
        assert!(text.contains(placeholder), "missing {placeholder}");
    }
}
