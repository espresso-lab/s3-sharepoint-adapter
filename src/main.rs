mod utils;

use std::path::Path;

use confique::Config;
use dotenv::dotenv;
use regex::Regex;
use salvo::http::StatusCode;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use tracing::warn;
use urlencoding::decode;
use utils::azure::{get_azure_object_data, head_azure_object, list_azure_objects, SearchRequest};
use utils::s3::generate_s3_list_objects_v2_response;

#[derive(Config)]
struct Conf {
    #[config(env = "APP_CLIENT_ID")]
    app_client_id: String,

    #[config(env = "APP_CLIENT_SECRET")]
    app_client_secret: String,

    #[config(env = "TENANT")]
    tenant: String,

    #[config(env = "SHAREPOINT_SITE_ID")]
    sharepoint_site_id: String,

    #[config(env = "FILENAME_PATTERN", default = "")]
    filename_pattern: String,

    #[config(env = "WHITELISTED_IPS")]
    whitelisted_ips: Option<String>,

    #[config(env = "API_TOKEN")]
    api_token: Option<String>,
}

fn config() -> &'static Conf {
    static CONFIG: OnceLock<Conf> = OnceLock::new();
    CONFIG.get_or_init(|| Conf::builder().env().load().unwrap())
}

#[derive(Deserialize, Serialize, Debug)]
struct SearchResult {
    file_name: String,
    file_path: String,
}

#[handler]
async fn ok_handler(res: &mut Response) {
    res.status_code(StatusCode::OK).render(Text::Plain("OK"))
}

#[handler]
async fn bad_request_handler(res: &mut Response) {
    res.status_code(StatusCode::BAD_REQUEST)
        .render(Text::Plain("BAD REQUEST"))
}

#[handler]
async fn head_handler(req: &mut Request, res: &mut Response) {
    let site_id = config().sharepoint_site_id.clone();

    let key = req.params().get("**path").cloned().unwrap_or_default();
    match head_azure_object(site_id.clone(), key.clone()).await {
        Ok(result) => {
            res.headers_mut()
                .insert("Content-Type", result.content_type.parse().unwrap());
            res.headers_mut()
                .insert("Content-Length", result.size.to_string().parse().unwrap());
            res.status_code(StatusCode::from_u16(result.status_code).unwrap());
        }
        Err(_) => {
            res.headers_mut()
                .insert("Content-Type", "application/xml".parse().unwrap());
            res.headers_mut()
                .insert("Content-Length", "0".parse().unwrap());
            res.status_code(StatusCode::NOT_FOUND);
        }
    }
}

#[handler]
async fn list_objects_v1(req: &mut Request, res: &mut Response) {
    let prefix = req
        .query::<String>("prefix")
        .unwrap_or("/".to_string())
        .trim_end_matches("/")
        .to_string();
    let max_keys = req.query::<u16>("max-keys").unwrap_or(1000);
    let site_id = config().sharepoint_site_id.clone();
    match list_azure_objects(site_id.clone(), prefix.clone(), max_keys, None).await {
        Ok(objects) => {
            res.status_code(StatusCode::OK).render(Text::Xml(
                generate_s3_list_objects_v2_response(site_id, prefix, objects, false),
            ));
        }
        Err(err) => {
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR)
                .render(Text::Plain(err.to_string()));
        }
    }
}

#[handler]
async fn search_handler(req: &mut Request, res: &mut Response) {
    let payload = req.parse_json::<SearchRequest>().await.unwrap();
    let site_id = config().sharepoint_site_id.clone();
    match list_azure_objects(
        site_id.clone(),
        payload.prefix.clone(),
        payload.max_keys.unwrap_or(1000),
        Some(payload.query),
    )
    .await
    {
        Ok(objects) => {
            let filename_pattern = config().filename_pattern.clone();
            let regex = Regex::new(&filename_pattern).unwrap();
            let search_results = objects
                .items
                .iter()
                .filter(|item| item.folder.is_none() && regex.is_match(&item.name))
                .map(|item| {
                    let web_url = decode(&item.web_url).expect("UTF-8").to_string();
                    let ending = web_url.split(&payload.prefix).last().unwrap_or_default();
                    let full = format!("{}{}", payload.prefix, ending);
                    let path = Path::new(full.as_str());
                    SearchResult {
                        file_name: path.file_name().unwrap().to_string_lossy().into_owned(),
                        file_path: path.parent().unwrap().display().to_string(),
                    }
                })
                .collect::<Vec<SearchResult>>();
            res.status_code(StatusCode::OK).render(Json(search_results));
        }
        Err(err) => {
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR)
                .render(Text::Plain(err.to_string()));
        }
    }
}

#[handler]
async fn get_object(req: &mut Request, res: &mut Response) {
    let filename_pattern = config().filename_pattern.clone();
    let regex = Regex::new(&filename_pattern).unwrap();
    let site_id = config().sharepoint_site_id.clone();
    let key = req.params().get("**path").cloned().unwrap_or_default();
    if !regex.is_match(&key) {
        res.status_code(StatusCode::FORBIDDEN);
        return;
    }
    match get_azure_object_data(site_id.clone(), key.clone()).await {
        Ok(result) => {
            res.headers_mut()
                .insert("Content-Type", result.content_type.parse().unwrap());
            res.headers_mut().insert(
                "Content-Disposition",
                format!("attachment; filename=\"{}\"", result.file_name)
                    .parse()
                    .unwrap(),
            );
            let _ = res.write_body(result.data);
        }
        Err(err) => {
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR)
                .render(Text::Plain(err.to_string()));
        }
    }
}

#[handler]
async fn auth_handler(req: &mut Request, res: &mut Response) {
    let whitelisted_ips = config().whitelisted_ips.clone();
    let req_ip = req
        .header::<String>("X-Forwarded-For")
        .unwrap_or("".to_string());

    let api_token = config().api_token.clone().expect("API Token not set");
    let req_token = req
        .header::<String>("Authorization")
        .unwrap_or("".to_string())
        .split(' ')
        .last()
        .unwrap_or("")
        .to_string();

    if whitelisted_ips
        .clone()
        .is_some_and(|ip| !ip.contains(&req_ip) || req_ip.is_empty())
    {
        warn!(
            "Invalid ip {}: {}",
            whitelisted_ips.unwrap_or("".to_string()),
            req_ip
        );
        res.status_code(StatusCode::FORBIDDEN);
        return;
    }

    if api_token.clone().ne(&req_token) {
        warn!("Invalid api token {}: {}", api_token, req_token);
        res.status_code(StatusCode::FORBIDDEN);
        return;
    }
}

#[tokio::main]
async fn main() {
    dotenv().ok();
    tracing_subscriber::fmt().init();

    let router = Router::new()
        .push(Router::with_path("status").get(ok_handler))
        .push(
            Router::new()
                .hoop(auth_handler)
                .push(Router::with_path("search").post(search_handler))
                .push(Router::with_path("<**path>").head(head_handler))
                .push(
                    Router::with_filter_fn(|req, _| {
                        req.query::<i8>("list-type").is_none()
                            && (req.query::<String>("prefix").is_some()
                                || (req.query::<String>("delimiter").is_some()
                                    || req.query::<String>("max-keys").is_some()))
                    })
                    .get(list_objects_v1),
                )
                .push(Router::with_path("<**path>").get(get_object)),
        )
        .goal(bad_request_handler);
    let service = Service::new(router).hoop(Logger::new());
    let acceptor = TcpListener::new("0.0.0.0:3000").bind().await;
    Server::new(acceptor).serve(service).await;
}
