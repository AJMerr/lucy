//! Making pictures: OpenRouter image models, and ComfyUI running a workflow.

use std::time::Duration;

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

fn openrouter(server: &FakeServer, base: &str) -> OpenRouterImages {
    OpenRouterImages::new(OpenAiCompatConfig {
        name: "pics".into(),
        base_url: format!("{}{base}", server.url()),
        model: "qwen/qwen-image-3-pro".into(),
        api_key: Some("sk-or-test".into()),
    })
}

/// OpenRouter's Image API answering with one picture.
fn picture_reply() -> FakeResponse {
    FakeResponse::json(
        200,
        json!({
            "created": 1748372400,
            "data": [{"b64_json": PNG, "media_type": "image/png"}],
            "usage": {"prompt_tokens": 0, "completion_tokens": 4175, "total_tokens": 4175, "cost": 0.04},
        })
        .to_string(),
    )
}

#[tokio::test]
async fn asks_openrouters_image_api_for_a_picture() {
    let server = FakeServer::start([picture_reply()]).await;

    let pictures = openrouter(&server, "/api/v1")
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
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.path, "/api/v1/images");
    assert_eq!(sent.headers["authorization"], "Bearer sk-or-test");
    assert_eq!(sent.body["model"], "qwen/qwen-image-3-pro");
    assert_eq!(sent.body["aspect_ratio"], "16:9");
    let prompt = sent.body["prompt"].as_str().unwrap();
    assert!(prompt.contains("a lighthouse at dusk"), "{prompt}");
    assert!(prompt.contains("people"), "{prompt}");
}

#[tokio::test]
async fn a_square_picture_sends_only_the_model_and_prompt() {
    let server = FakeServer::start([picture_reply()]).await;

    openrouter(&server, "/api/v1")
        .generate(&request("a fox"))
        .await
        .unwrap();

    let [sent] = server.requests().try_into().unwrap();
    assert_eq!(
        sent.body,
        json!({"model": "qwen/qwen-image-3-pro", "prompt": "a fox"})
    );
}

#[tokio::test]
async fn an_endpoint_given_as_the_image_api_itself_works_too() {
    let server = FakeServer::start([picture_reply()]).await;

    openrouter(&server, "/api/v1/images/")
        .generate(&request("a fox"))
        .await
        .unwrap();

    assert_eq!(server.requests()[0].path, "/api/v1/images");
}

#[tokio::test]
async fn a_reply_without_a_picture_is_an_error() {
    let empty = FakeResponse::json(200, json!({"created": 1, "data": []}).to_string());
    let server = FakeServer::start([empty]).await;

    let err = openrouter(&server, "/api/v1")
        .generate(&request("a fox"))
        .await
        .unwrap_err();

    assert!(err.to_string().contains("no picture"), "{err}");
}

#[tokio::test]
async fn openrouters_reason_for_refusing_comes_through() {
    let refused = FakeResponse::json(
        400,
        json!({"error": {"code": 400, "message": "aspect_ratio 21:9 is not supported"}})
            .to_string(),
    );
    let server = FakeServer::start([refused]).await;

    let err = openrouter(&server, "/api/v1")
        .generate(&request("a fox"))
        .await
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("aspect_ratio 21:9 is not supported"),
        "{err}"
    );
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
