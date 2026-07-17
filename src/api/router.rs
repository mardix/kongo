//! HTTP router construction, including routes and response compression middleware.

use axum::{
    Router,
    body::Body,
    extract::DefaultBodyLimit,
    extract::State,
    http::{HeaderName, HeaderValue, Method, Request, header},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use tower_http::{
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
    services::{ServeDir, ServeFile},
};

use crate::{
    api::handlers::{docs, gateway, meta_operations, ping, validate_access_key},
    state::AppState,
};

pub fn build_router(state: AppState, max_request_bytes: usize) -> Router {
    let base = state.base_path.clone();
    let mut router = Router::new()
        .route(full_path(&base, "/ping").as_str(), get(ping))
        .route(
            full_path(&base, "/meta/operations").as_str(),
            get(meta_operations),
        )
        .route(full_path(&base, "/gateway").as_str(), post(gateway));

    let mut browser_routes = Router::new();
    let mut has_browser_routes = false;

    if state.docs_enabled {
        browser_routes = browser_routes.route(full_path(&base, "/doc").as_str(), get(docs));
        has_browser_routes = true;
    }

    if state.admin_ui_enabled {
        let admin_path = full_path(&base, "/admin");
        let admin_slash_path = format!("{admin_path}/");
        let admin_dir = state.admin_ui_dir.clone();
        let index_file = format!("{admin_dir}/index.html");
        let admin_service = ServeDir::new(admin_dir).fallback(ServeFile::new(index_file));
        let redirect_target = admin_slash_path.clone();
        browser_routes = browser_routes
            .route(
                admin_path.as_str(),
                get(move || {
                    let target = redirect_target.clone();
                    async move { Redirect::permanent(&target) }
                }),
            )
            .nest_service(admin_slash_path.as_str(), admin_service);
        has_browser_routes = true;
    }

    if has_browser_routes {
        browser_routes = browser_routes.route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_browser_auth,
        ));
        router = router.merge(browser_routes);
    }

    let router = router
        .layer(DefaultBodyLimit::max(max_request_bytes.max(1024)))
        .layer(CompressionLayer::new());

    let router = if let Some(cors) = cors_layer(&state.cors_allowed_origins) {
        router.layer(cors)
    } else {
        router
    };

    router.with_state(state)
}

fn full_path(base: &str, suffix: &str) -> String {
    if base.is_empty() {
        suffix.to_string()
    } else {
        format!("{base}{suffix}")
    }
}

fn cors_layer(origins: &[String]) -> Option<CorsLayer> {
    if origins.is_empty() {
        return None;
    }

    let mut layer = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            HeaderName::from_static("x-access-key"),
        ]);

    if origins.iter().any(|origin| origin == "*") {
        layer = layer.allow_origin(Any);
    } else {
        let parsed = origins
            .iter()
            .filter_map(|origin| origin.parse::<HeaderValue>().ok())
            .collect::<Vec<HeaderValue>>();
        if parsed.is_empty() {
            return None;
        }
        layer = layer.allow_origin(parsed);
    }

    Some(layer)
}

async fn require_browser_auth(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    match validate_access_key(&state, request.headers()) {
        Ok(()) => next.run(request).await,
        Err(err) => {
            let mut response = err.into_response();
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Basic realm=\"Kongodb\", charset=\"UTF-8\""),
            );
            response
        }
    }
}
