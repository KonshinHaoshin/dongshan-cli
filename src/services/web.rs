use axum::Router;
use axum::extract::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};

use crate::config::{AutoExecMode, ModelApiProvider};
use crate::services::{diagnostics, models, prompts, settings};

const INDEX_HTML: &str = include_str!("../../web/index.html");
const APP_JS: &str = include_str!("../../web/app.js");
const APP_CSS: &str = include_str!("../../web/app.css");

pub fn router() -> Router {
    Router::new()
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
        .route("/api/policy", post(api_policy_update))
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn asset_js() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/javascript; charset=utf-8"),
        )],
        APP_JS,
    )
        .into_response()
}

async fn asset_css() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/css; charset=utf-8"),
        )],
        APP_CSS,
    )
        .into_response()
}

async fn api_state() -> ApiResult<Json<StateResponse>> {
    let mut cfg = settings::load().map_err(api_err)?;
    models::ensure_catalog(&mut cfg);
    let prompt_list = prompts::list_docs()
        .map_err(api_err)?
        .into_iter()
        .map(|(name, content)| PromptEntry { name, content })
        .collect::<Vec<_>>();

    Ok(Json(StateResponse {
        config: ConfigSummary {
            base_url: cfg.base_url.clone(),
            model: cfg.model.clone(),
            active_provider: cfg
                .model_profiles
                .get(&cfg.model)
                .map(|profile| profile.provider)
                .unwrap_or(ModelApiProvider::Openai),
            api_key_env: cfg.api_key_env.clone(),
            api_key_set: cfg.api_key.as_ref().is_some_and(|value| !value.trim().is_empty()),
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
        last_diagnostic: diagnostics::last_error(),
    }))
}

async fn api_set_config(Json(req): Json<ConfigUpdateRequest>) -> ApiResult<Json<SimpleOk>> {
    let mut cfg = settings::load().map_err(api_err)?;
    if let Some(value) = req.model {
        models::set_active(&mut cfg, &value);
    }
    if let Some(value) = req.base_url {
        cfg.base_url = value;
    }
    if let Some(value) = req.api_key_env {
        cfg.api_key_env = value;
    }
    if let Some(value) = req.api_key {
        cfg.api_key = if value.trim().is_empty() { None } else { Some(value) };
    }
    if let Some(value) = req.allow_nsfw {
        cfg.allow_nsfw = value;
    }
    if let Some(provider) = req.provider {
        let active_model = cfg.model.clone();
        models::upsert_profile(&mut cfg, &active_model, None, None, None, Some(provider));
    }
    models::ensure_catalog(&mut cfg);
    settings::save(&cfg).map_err(api_err)?;
    Ok(Json(SimpleOk { ok: true }))
}

async fn api_prompt_save(Json(req): Json<PromptSaveRequest>) -> ApiResult<Json<SimpleOk>> {
    prompts::save(&req.name, &req.content).map_err(api_err)?;
    Ok(Json(SimpleOk { ok: true }))
}

async fn api_prompt_use(Json(req): Json<PromptUseRequest>) -> ApiResult<Json<SimpleOk>> {
    let mut cfg = settings::load().map_err(api_err)?;
    cfg.active_prompt = req.name;
    settings::save(&cfg).map_err(api_err)?;
    Ok(Json(SimpleOk { ok: true }))
}

async fn api_prompt_delete(Json(req): Json<PromptDeleteRequest>) -> ApiResult<Json<SimpleOk>> {
    let mut cfg = settings::load().map_err(api_err)?;
    prompts::remove(&req.name).map_err(api_err)?;
    if cfg.active_prompt == req.name {
        cfg.active_prompt = "default".to_string();
        settings::save(&cfg).map_err(api_err)?;
    }
    Ok(Json(SimpleOk { ok: true }))
}

async fn api_model_add(Json(req): Json<ModelAddRequest>) -> ApiResult<Json<SimpleOk>> {
    let mut cfg = settings::load().map_err(api_err)?;
    models::add_with_profile(&mut cfg, &req.name);
    if req.provider.is_some()
        || req.base_url.is_some()
        || req.api_key_env.is_some()
        || req.api_key.is_some()
    {
        models::upsert_profile(
            &mut cfg,
            &req.name,
            req.base_url,
            req.api_key_env,
            req.api_key,
            req.provider,
        );
    }
    settings::save(&cfg).map_err(api_err)?;
    Ok(Json(SimpleOk { ok: true }))
}

async fn api_model_use(Json(req): Json<ModelUseRequest>) -> ApiResult<Json<SimpleOk>> {
    let mut cfg = settings::load().map_err(api_err)?;
    models::set_active(&mut cfg, &req.name);
    settings::save(&cfg).map_err(api_err)?;
    Ok(Json(SimpleOk { ok: true }))
}

async fn api_model_remove(Json(req): Json<ModelRemoveRequest>) -> ApiResult<Json<SimpleOk>> {
    let mut cfg = settings::load().map_err(api_err)?;
    if cfg.model == req.name {
        return Err((
            StatusCode::BAD_REQUEST,
            "Cannot remove active model".to_string(),
        ));
    }
    if !models::remove(&mut cfg, &req.name) {
        return Err((StatusCode::BAD_REQUEST, "Model not found".to_string()));
    }
    settings::save(&cfg).map_err(api_err)?;
    Ok(Json(SimpleOk { ok: true }))
}

async fn api_policy_update(Json(req): Json<PolicyUpdateRequest>) -> ApiResult<Json<SimpleOk>> {
    let mut cfg = settings::load().map_err(api_err)?;
    if let Some(value) = req.auto_exec_mode {
        cfg.auto_exec_mode = value;
    }
    if let Some(value) = req.auto_exec_allow {
        cfg.auto_exec_allow = value;
    }
    if let Some(value) = req.auto_exec_deny {
        cfg.auto_exec_deny = value;
    }
    if let Some(value) = req.auto_confirm_exec {
        cfg.auto_confirm_exec = value;
    }
    if let Some(value) = req.auto_exec_trusted {
        cfg.auto_exec_trusted = value;
    }
    settings::save(&cfg).map_err(api_err)?;
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
    last_diagnostic: Option<crate::diagnostics::LastDiagnostic>,
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
    active_provider: ModelApiProvider,
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
    provider: Option<ModelApiProvider>,
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
    provider: Option<ModelApiProvider>,
    base_url: Option<String>,
    api_key_env: Option<String>,
    api_key: Option<String>,
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
