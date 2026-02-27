use std::net::SocketAddr;

use anyhow::Result;
use axum::extract::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};

use crate::config::{
    AutoExecMode, add_model_with_active_profile, ensure_model_catalog, load_config_or_default,
    remove_model, save_config, set_active_model, update_active_model_profile,
};
use crate::prompt_store::{list_prompts, remove_prompt, save_prompt};

const INDEX_HTML: &str = include_str!("../web/index.html");
const APP_JS: &str = include_str!("../web/app.js");
const APP_CSS: &str = include_str!("../web/app.css");

pub async fn run_web(port: u16) -> Result<()> {
    let app = Router::new()
        .route("/", get(index))
        .route("/assets/app.js", get(asset_js))
        .route("/assets/app.css", get(asset_css))
        .route("/api/state", get(api_state))
        .route("/api/config", post(api_set_config))
        .route("/api/prompt/save", post(api_prompt_save))
        .route("/api/prompt/use", post(api_prompt_use))
        .route("/api/prompt/delete", post(api_prompt_delete))
        .route("/api/model/add", post(api_model_add))
        .route("/api/model/use", post(api_model_use))
        .route("/api/model/remove", post(api_model_remove))
        .route("/api/policy", post(api_policy_update));

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    println!("dongshan web running at http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn asset_js() -> Response {
    (
        [(header::CONTENT_TYPE, HeaderValue::from_static("application/javascript; charset=utf-8"))],
        APP_JS,
    )
        .into_response()
}

async fn asset_css() -> Response {
    (
        [(header::CONTENT_TYPE, HeaderValue::from_static("text/css; charset=utf-8"))],
        APP_CSS,
    )
        .into_response()
}

async fn api_state() -> ApiResult<Json<StateResponse>> {
    let mut cfg = load_config_or_default().map_err(api_err)?;
    ensure_model_catalog(&mut cfg);
    let prompts = list_prompts().map_err(api_err)?;
    let prompt_list = prompts
        .into_iter()
        .map(|p| PromptEntry {
            name: p.name().to_string(),
            content: p.content().to_string(),
        })
        .collect::<Vec<_>>();

    Ok(Json(StateResponse {
        config: ConfigSummary {
            base_url: cfg.base_url.clone(),
            model: cfg.model.clone(),
            api_key_env: cfg.api_key_env.clone(),
            api_key_set: cfg.api_key.as_ref().is_some_and(|v| !v.trim().is_empty()),
            active_prompt: cfg.active_prompt.clone(),
            allow_nsfw: cfg.allow_nsfw,
            auto_exec_mode: cfg.auto_exec_mode,
            auto_exec_allow: cfg.auto_exec_allow.clone(),
            auto_exec_deny: cfg.auto_exec_deny.clone(),
            auto_confirm_exec: cfg.auto_confirm_exec,
            auto_exec_trusted: cfg.auto_exec_trusted.clone(),
            model_catalog: cfg.model_catalog.clone(),
        },
        prompts: prompt_list,
    }))
}

async fn api_set_config(Json(req): Json<ConfigUpdateRequest>) -> ApiResult<Json<SimpleOk>> {
    let mut cfg = load_config_or_default().map_err(api_err)?;
    if let Some(v) = req.model {
        set_active_model(&mut cfg, &v);
    }
    if let Some(v) = req.base_url {
        cfg.base_url = v;
    }
    if let Some(v) = req.api_key_env {
        cfg.api_key_env = v;
    }
    if let Some(v) = req.api_key {
        cfg.api_key = if v.trim().is_empty() { None } else { Some(v) };
    }
    if let Some(v) = req.allow_nsfw {
        cfg.allow_nsfw = v;
    }
    update_active_model_profile(&mut cfg);
    ensure_model_catalog(&mut cfg);
    save_config(&cfg).map_err(api_err)?;
    Ok(Json(SimpleOk { ok: true }))
}

async fn api_prompt_save(Json(req): Json<PromptSaveRequest>) -> ApiResult<Json<SimpleOk>> {
    save_prompt(&req.name, &req.content).map_err(api_err)?;
    Ok(Json(SimpleOk { ok: true }))
}

async fn api_prompt_use(Json(req): Json<PromptUseRequest>) -> ApiResult<Json<SimpleOk>> {
    let mut cfg = load_config_or_default().map_err(api_err)?;
    cfg.active_prompt = req.name;
    save_config(&cfg).map_err(api_err)?;
    Ok(Json(SimpleOk { ok: true }))
}

async fn api_prompt_delete(Json(req): Json<PromptDeleteRequest>) -> ApiResult<Json<SimpleOk>> {
    let mut cfg = load_config_or_default().map_err(api_err)?;
    remove_prompt(&req.name).map_err(api_err)?;
    if cfg.active_prompt == req.name {
        cfg.active_prompt = "default".to_string();
        save_config(&cfg).map_err(api_err)?;
    }
    Ok(Json(SimpleOk { ok: true }))
}

async fn api_model_add(Json(req): Json<ModelAddRequest>) -> ApiResult<Json<SimpleOk>> {
    let mut cfg = load_config_or_default().map_err(api_err)?;
    add_model_with_active_profile(&mut cfg, &req.name);
    save_config(&cfg).map_err(api_err)?;
    Ok(Json(SimpleOk { ok: true }))
}

async fn api_model_use(Json(req): Json<ModelUseRequest>) -> ApiResult<Json<SimpleOk>> {
    let mut cfg = load_config_or_default().map_err(api_err)?;
    set_active_model(&mut cfg, &req.name);
    save_config(&cfg).map_err(api_err)?;
    Ok(Json(SimpleOk { ok: true }))
}

async fn api_model_remove(Json(req): Json<ModelRemoveRequest>) -> ApiResult<Json<SimpleOk>> {
    let mut cfg = load_config_or_default().map_err(api_err)?;
    if cfg.model == req.name {
        return Err((StatusCode::BAD_REQUEST, "Cannot remove active model".to_string()));
    }
    remove_model(&mut cfg, &req.name);
    save_config(&cfg).map_err(api_err)?;
    Ok(Json(SimpleOk { ok: true }))
}

async fn api_policy_update(Json(req): Json<PolicyUpdateRequest>) -> ApiResult<Json<SimpleOk>> {
    let mut cfg = load_config_or_default().map_err(api_err)?;
    if let Some(v) = req.auto_exec_mode {
        cfg.auto_exec_mode = v;
    }
    if let Some(v) = req.auto_exec_allow {
        cfg.auto_exec_allow = v;
    }
    if let Some(v) = req.auto_exec_deny {
        cfg.auto_exec_deny = v;
    }
    if let Some(v) = req.auto_confirm_exec {
        cfg.auto_confirm_exec = v;
    }
    if let Some(v) = req.auto_exec_trusted {
        cfg.auto_exec_trusted = v;
    }
    save_config(&cfg).map_err(api_err)?;
    Ok(Json(SimpleOk { ok: true }))
}

type ApiResult<T> = std::result::Result<T, (StatusCode, String)>;

fn api_err(err: anyhow::Error) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, err.to_string())
}

#[derive(Debug, Serialize)]
struct SimpleOk {
    ok: bool,
}

#[derive(Debug, Serialize)]
struct StateResponse {
    config: ConfigSummary,
    prompts: Vec<PromptEntry>,
}

#[derive(Debug, Serialize)]
struct PromptEntry {
    name: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct ConfigSummary {
    base_url: String,
    model: String,
    api_key_env: String,
    api_key_set: bool,
    active_prompt: String,
    allow_nsfw: bool,
    auto_exec_mode: AutoExecMode,
    auto_exec_allow: Vec<String>,
    auto_exec_deny: Vec<String>,
    auto_confirm_exec: bool,
    auto_exec_trusted: Vec<String>,
    model_catalog: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ConfigUpdateRequest {
    base_url: Option<String>,
    model: Option<String>,
    api_key_env: Option<String>,
    api_key: Option<String>,
    allow_nsfw: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct PromptSaveRequest {
    name: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct PromptUseRequest {
    name: String,
}

#[derive(Debug, Deserialize)]
struct PromptDeleteRequest {
    name: String,
}

#[derive(Debug, Deserialize)]
struct ModelAddRequest {
    name: String,
}

#[derive(Debug, Deserialize)]
struct ModelUseRequest {
    name: String,
}

#[derive(Debug, Deserialize)]
struct ModelRemoveRequest {
    name: String,
}

#[derive(Debug, Deserialize)]
struct PolicyUpdateRequest {
    auto_exec_mode: Option<AutoExecMode>,
    auto_exec_allow: Option<Vec<String>>,
    auto_exec_deny: Option<Vec<String>>,
    auto_confirm_exec: Option<bool>,
    auto_exec_trusted: Option<Vec<String>>,
}
