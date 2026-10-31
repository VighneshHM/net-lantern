//! The React UI, embedded in the binary, and the router fallback that serves
//! it.
//!
//! The UI is a single-page app: any path that is not a built file answers with
//! `index.html` so client-side routes survive a reload. Paths under `/api`
//! never do, so a mistyped API call still gets a JSON error instead of HTML.
//! Without a build (`ui/dist` missing) a placeholder page explains what to run.
//!
//! The guard wraps this handler like every route, so UI paths also pass the
//! Host and Origin checks (but need no session).

use std::borrow::Cow;
use std::path::{Component, Path};

use axum::http::header::{ALLOW, CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, Method, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

use crate::error::ApiError;

/// `ui/dist` as built by `npm run build`. Missing is allowed so the server
/// compiles (and serves the placeholder) before the UI has ever been built;
/// `build.rs` makes cargo re-embed once it has.
#[derive(RustEmbed)]
#[folder = "../../ui/dist"]
#[allow_missing = true]
struct UiAssets;

/// The app shell, and what every unknown non-API path answers with.
const INDEX: &str = "index.html";

/// Served as the shell when `ui/dist` was not embedded.
const PLACEHOLDER: &str = "<!doctype html>\n\
<html lang=\"en\">\n\
<head><meta charset=\"utf-8\"><title>NetBot</title></head>\n\
<body><p>The NetBot UI isn't built yet. Run `npm run build` in `ui/`, then rebuild.</p></body>\n\
</html>\n";

/// Vite's hashed bundles live here; a changed file gets a new name.
const HASHED_DIR: &str = "assets/";

/// The router's fallback: anything no API route matched.
pub(crate) async fn fallback(method: Method, uri: Uri) -> Response {
    respond(&method, uri.path(), |path| {
        UiAssets::get(path).map(|file| file.data)
    })
}

/// The decision, with the embedded files behind `lookup` (a path relative to
/// `ui/dist` to its bytes) so it can be tested without a UI build.
fn respond(
    method: &Method,
    path: &str,
    lookup: impl Fn(&str) -> Option<Cow<'static, [u8]>>,
) -> Response {
    if is_api_path(path) {
        return ApiError::not_found().into_response();
    }
    if method != Method::GET && method != Method::HEAD {
        let mut response = ApiError::method_not_allowed().into_response();
        response
            .headers_mut()
            .insert(ALLOW, HeaderValue::from_static("GET, HEAD"));
        return response;
    }

    let wanted = path.strip_prefix('/').unwrap_or(path);
    let wanted = if wanted.is_empty() { INDEX } else { wanted };
    if is_plain_relative(wanted) {
        if let Some(bytes) = lookup(wanted) {
            return asset(wanted, bytes);
        }
    }
    let shell = lookup(INDEX).unwrap_or(Cow::Borrowed(PLACEHOLDER.as_bytes()));
    asset(INDEX, shell)
}

/// Whether `path` is a name below `ui/dist`: every component a plain name, so
/// no `..`, `.`, leading `/` or drive/UNC prefix, and no `\` or `:` either
/// (which Windows treats as separators and stream/drive markers). Anything
/// else is not looked up at all: in debug builds the lookup reads the
/// filesystem, and `Path::join` with an absolute path replaces the base
/// folder instead of extending it.
fn is_plain_relative(path: &str) -> bool {
    !path.contains(['\\', ':'])
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

/// `/api` itself or anything below it. `/apiary` is a UI path. The guard
/// draws the same line for its session check.
fn is_api_path(path: &str) -> bool {
    path == "/api" || path.starts_with("/api/")
}

fn asset(path: &str, bytes: Cow<'static, [u8]>) -> Response {
    let mut response = ([(CONTENT_TYPE, content_type(path))], bytes).into_response();
    if let Some(cache) = cache_control(path) {
        response
            .headers_mut()
            .insert(CACHE_CONTROL, HeaderValue::from_static(cache));
    }
    response
}

fn content_type(path: &str) -> String {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    if mime.essence_str() == "text/html" {
        "text/html; charset=utf-8".to_string()
    } else {
        mime.to_string()
    }
}

/// The shell is revalidated on every load so a rebuilt UI is picked up;
/// hashed bundles never change under their name. Anything else gets no
/// caching header.
fn cache_control(path: &str) -> Option<&'static str> {
    if path == INDEX {
        Some("no-cache")
    } else if path.starts_with(HASHED_DIR) {
        Some("public, max-age=31536000, immutable")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use axum::body::to_bytes;
    use axum::http::{HeaderMap, HeaderName, StatusCode};

    const SHELL: &[u8] = b"<!doctype html><html><body>shell</body></html>";
    const BUNDLE: &[u8] = b"console.log('hi')";

    /// A fake `ui/dist` with a shell, a hashed bundle, an icon and a file with
    /// an extension nobody knows.
    fn dist(path: &str) -> Option<Cow<'static, [u8]>> {
        match path {
            "index.html" => Some(Cow::Borrowed(SHELL)),
            "assets/app-1a2b3c.js" => Some(Cow::Borrowed(BUNDLE)),
            "favicon.svg" => Some(Cow::Borrowed(b"<svg/>")),
            "data.nbxyz" => Some(Cow::Borrowed(b"?")),
            _ => None,
        }
    }

    /// A `ui/dist` that was never built.
    fn empty(_: &str) -> Option<Cow<'static, [u8]>> {
        None
    }

    /// A `ui/dist` that must not be touched.
    fn untouchable(path: &str) -> Option<Cow<'static, [u8]>> {
        panic!("looked up {path:?} but should not have");
    }

    async fn parts(response: Response) -> (StatusCode, HeaderMap, Vec<u8>) {
        let (head, body) = response.into_parts();
        let bytes = to_bytes(body, usize::MAX).await.unwrap().to_vec();
        (head.status, head.headers, bytes)
    }

    fn header(headers: &HeaderMap, name: HeaderName) -> Option<&str> {
        headers.get(name).map(|v| v.to_str().unwrap())
    }

    #[test]
    fn only_api_and_its_children_are_api_paths() {
        for path in ["/api", "/api/", "/api/nope", "/api/scans/1"] {
            assert!(is_api_path(path), "{path}");
        }
        for path in ["/", "/apiary", "/api2/x", "/API", "/x/api", "/devices"] {
            assert!(!is_api_path(path), "{path}");
        }
    }

    #[test]
    fn content_types_follow_the_extension() {
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(content_type("assets/app.css"), "text/css");
        assert!(content_type("assets/app.js").contains("javascript"));
        assert_eq!(content_type("favicon.svg"), "image/svg+xml");
        assert_eq!(content_type("data.nbxyz"), "application/octet-stream");
        assert_eq!(content_type("README"), "application/octet-stream");
    }

    #[test]
    fn only_the_shell_and_hashed_bundles_get_cache_headers() {
        assert_eq!(cache_control("index.html"), Some("no-cache"));
        assert_eq!(
            cache_control("assets/app-1a2b3c.js"),
            Some("public, max-age=31536000, immutable")
        );
        assert_eq!(cache_control("favicon.svg"), None);
        assert_eq!(cache_control("sub/index.html"), None);
        assert_eq!(cache_control("assetsx/app.js"), None);
    }

    #[tokio::test]
    async fn a_built_file_is_served_with_its_type_and_the_hashed_cache_header() {
        let response = respond(&Method::GET, "/assets/app-1a2b3c.js", dist);
        let (status, head, body) = parts(response).await;

        assert_eq!(status, StatusCode::OK);
        assert!(header(&head, CONTENT_TYPE).unwrap().contains("javascript"));
        assert_eq!(
            header(&head, CACHE_CONTROL),
            Some("public, max-age=31536000, immutable")
        );
        assert_eq!(body, BUNDLE);
    }

    #[tokio::test]
    async fn a_file_without_a_cache_rule_has_no_cache_header() {
        let (status, head, _) = parts(respond(&Method::GET, "/favicon.svg", dist)).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(header(&head, CONTENT_TYPE), Some("image/svg+xml"));
        assert_eq!(header(&head, CACHE_CONTROL), None);
    }

    #[tokio::test]
    async fn an_unknown_file_type_is_an_octet_stream() {
        let (_, head, body) = parts(respond(&Method::GET, "/data.nbxyz", dist)).await;

        assert_eq!(
            header(&head, CONTENT_TYPE),
            Some("application/octet-stream")
        );
        assert_eq!(body, b"?");
    }

    #[tokio::test]
    async fn the_root_and_unknown_paths_get_the_shell() {
        for path in ["/", "/devices", "/devices/abc", "/assets/missing.js", "//x"] {
            let (status, head, body) = parts(respond(&Method::GET, path, dist)).await;

            assert_eq!(status, StatusCode::OK, "{path}");
            assert_eq!(
                header(&head, CONTENT_TYPE),
                Some("text/html; charset=utf-8"),
                "{path}"
            );
            assert_eq!(header(&head, CACHE_CONTROL), Some("no-cache"), "{path}");
            assert_eq!(body, SHELL, "{path}");
        }
    }

    /// GETs `path` over a `ui/dist` that holds only a shell (or nothing, if
    /// `built` is false) and records every path the handler looked up.
    async fn get_recording_lookups(
        path: &str,
        built: bool,
    ) -> (StatusCode, HeaderMap, Vec<u8>, Vec<String>) {
        let seen = RefCell::new(Vec::new());
        let response = respond(&Method::GET, path, |wanted| {
            seen.borrow_mut().push(wanted.to_string());
            (built && wanted == INDEX).then_some(Cow::Borrowed(SHELL))
        });
        let (status, headers, body) = parts(response).await;
        (status, headers, body, seen.into_inner())
    }

    #[tokio::test]
    async fn paths_that_are_not_plain_relative_names_never_reach_the_files() {
        // Absolute, drive-letter, UNC-looking and parent-directory inputs
        // would otherwise escape `ui/dist` (an absolute path replaces the
        // base in `Path::join`) and hit the filesystem in debug builds.
        for path in [
            "/../x",
            "/a/../../x",
            "/a/../b",
            "/./x",
            "//etc/x",
            "///h/s/x",
            "/C:/x",
            "/a:b",
            "/a\\b",
            "/..\\x",
        ] {
            let (status, head, body, seen) = get_recording_lookups(path, true).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            assert_eq!(body, SHELL, "{path}");
            assert_eq!(header(&head, CACHE_CONTROL), Some("no-cache"), "{path}");
            assert_eq!(seen, [INDEX], "{path}: only the shell may be looked up");

            let (status, _, body, seen) = get_recording_lookups(path, false).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            assert!(
                String::from_utf8(body).unwrap().contains("isn't built yet"),
                "{path}"
            );
            assert_eq!(seen, [INDEX], "{path}: only the shell may be looked up");
        }
    }

    #[tokio::test]
    async fn ordinary_nested_names_still_reach_the_files() {
        for path in [
            "/assets/app-1a2b3c.js",
            "/assets/inter-latin.woff2",
            "/favicon.svg",
            "/a/b/c.d.e",
        ] {
            let (_, _, _, seen) = get_recording_lookups(path, true).await;
            assert_eq!(seen.first().map(String::as_str), path.strip_prefix('/'));
        }
    }

    #[tokio::test]
    async fn head_is_treated_like_get() {
        let (status, head, _) = parts(respond(&Method::HEAD, "/devices", dist)).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            header(&head, CONTENT_TYPE),
            Some("text/html; charset=utf-8")
        );
    }

    #[tokio::test]
    async fn without_a_build_every_ui_path_gets_the_placeholder() {
        for path in ["/", "/devices", "/index.html"] {
            let (status, head, body) = parts(respond(&Method::GET, path, empty)).await;
            let page = String::from_utf8(body).unwrap();

            assert_eq!(status, StatusCode::OK, "{path}");
            assert_eq!(
                header(&head, CONTENT_TYPE),
                Some("text/html; charset=utf-8"),
                "{path}"
            );
            assert_eq!(header(&head, CACHE_CONTROL), Some("no-cache"), "{path}");
            assert!(
                page.contains(
                    "The NetBot UI isn't built yet. Run `npm run build` in `ui/`, then rebuild."
                ),
                "{path}: {page}"
            );
            assert!(page.contains("<html"), "{page}");
        }
    }

    #[tokio::test]
    async fn api_paths_are_json_404s_whatever_the_method_and_never_touch_the_files() {
        for method in [Method::GET, Method::POST, Method::DELETE] {
            for path in ["/api", "/api/nope"] {
                let (status, head, body) = parts(respond(&method, path, untouchable)).await;

                assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}");
                assert_eq!(
                    header(&head, CONTENT_TYPE),
                    Some("application/json"),
                    "{method} {path}"
                );
                let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(json["code"], "not_found");
            }
        }
    }

    #[tokio::test]
    async fn other_methods_on_ui_paths_are_405_json_and_never_touch_the_files() {
        for method in [Method::POST, Method::PUT, Method::DELETE, Method::PATCH] {
            let (status, head, body) = parts(respond(&method, "/devices", untouchable)).await;

            assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{method}");
            assert_eq!(header(&head, ALLOW), Some("GET, HEAD"), "{method}");
            assert_eq!(
                header(&head, CONTENT_TYPE),
                Some("application/json"),
                "{method}"
            );
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(json["code"], "method_not_allowed");
        }
    }
}
